//! Session management for active Cardputer connections

use crate::capture::{CapturedFrame, ScreenCapturer};
use crate::config::Config;
use crate::crypto::CryptoContext;
use crate::input::InputController;
use crate::network::{Connection, NetworkError};
use crate::protocol::{
    InputMode, KeyEvent, ModeSwitch, MouseClick, MouseMove, PacketType, ScreenFrame,
};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::time::interval;
use tracing::{debug, error, info, warn};

/// Active session with a Cardputer
pub struct Session {
    connection: Connection,
    addr: SocketAddr,
    config: Arc<Config>,
    start_time: Instant,
    last_activity: Arc<AtomicU64>,
    running: Arc<AtomicBool>,
}

impl Session {
    pub fn new(
        stream: TcpStream,
        addr: SocketAddr,
        crypto: CryptoContext,
        config: Arc<Config>,
    ) -> Self {
        Self {
            connection: Connection::new(stream, crypto),
            addr,
            config,
            start_time: Instant::now(),
            last_activity: Arc::new(AtomicU64::new(0)),
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Get session address
    pub fn addr(&self) -> SocketAddr {
        self.addr
    }

    /// Get session uptime in milliseconds
    pub fn uptime_ms(&self) -> u64 {
        self.start_time.elapsed().as_millis() as u64
    }

    /// Update last activity timestamp
    fn touch(&self) {
        self.last_activity
            .store(self.uptime_ms(), Ordering::Relaxed);
    }

    /// Check if session has timed out
    pub fn is_timed_out(&self) -> bool {
        let timeout_secs = self.config.server.session_timeout_secs;
        if timeout_secs == 0 {
            return false; // Never timeout
        }

        let last = self.last_activity.load(Ordering::Relaxed);
        let now = self.uptime_ms();
        let idle_ms = now - last;

        idle_ms > timeout_secs * 1000
    }

    /// Check if session is running
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// Stop the session
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    /// Run the session event loop
    pub async fn run(&mut self) -> Result<(), NetworkError> {
        info!("Session started with {}", self.addr);

        // Send session start
        self.connection
            .send(PacketType::SessionStart, &[])
            .await?;

        // Initialize input controller
        let mut input = match InputController::new() {
            Ok(i) => i,
            Err(e) => {
                error!("Failed to initialize input controller: {}", e);
                return Err(NetworkError::IoError(std::io::Error::new(
                    std::io::ErrorKind::Other,
                    e.to_string(),
                )));
            }
        };

        // Create channels for async communication
        let (frame_tx, mut frame_rx) = mpsc::channel::<CapturedFrame>(2);

        // Frame capture interval
        let frame_interval = Duration::from_millis(self.config.get_frame_interval_ms());

        // Capture config parameters to pass into blocking task
        let target_width = self.config.display.target_width;
        let target_height = self.config.display.target_height;
        let jpeg_quality = self.config.server.jpeg_quality;
        let capture_region = self.config.display.capture_region;

        // Spawn frame capture task - create ScreenCapturer inside to avoid Send issues
        let running = self.running.clone();
        let capture_handle = tokio::task::spawn_blocking(move || {
            // Create ScreenCapturer inside the blocking task (scrap::Capturer is not Send)
            let mut capturer = match ScreenCapturer::new(
                target_width,
                target_height,
                jpeg_quality,
                capture_region,
            ) {
                Ok(c) => c,
                Err(e) => {
                    error!("Failed to initialize screen capturer: {}", e);
                    return;
                }
            };

            while running.load(Ordering::Relaxed) {
                match capturer.capture_frame() {
                    Ok(Some(frame)) => {
                        if frame_tx.blocking_send(frame).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {
                        // No change, skip
                    }
                    Err(e) => {
                        warn!("Capture error: {}", e);
                    }
                }
                std::thread::sleep(frame_interval);
            }
        });

        // Heartbeat interval
        let mut heartbeat_interval = interval(Duration::from_secs(5));

        // Timeout check interval
        let mut timeout_interval = interval(Duration::from_secs(1));

        self.touch();

        loop {
            tokio::select! {
                // Handle incoming packets
                result = self.connection.receive() => {
                    match result {
                        Ok((packet_type, payload)) => {
                            self.touch();

                            match self.handle_packet(packet_type, &payload, &mut input).await {
                                Ok(should_continue) => {
                                    if !should_continue {
                                        info!("Session ended by client");
                                        break;
                                    }
                                }
                                Err(e) => {
                                    error!("Packet handling error: {}", e);
                                    break;
                                }
                            }
                        }
                        Err(NetworkError::ConnectionClosed) => {
                            info!("Client disconnected");
                            break;
                        }
                        Err(e) => {
                            error!("Receive error: {}", e);
                            break;
                        }
                    }
                }

                // Send captured frames
                Some(frame) = frame_rx.recv() => {
                    if let Err(e) = self.send_frame(frame).await {
                        error!("Failed to send frame: {}", e);
                        break;
                    }
                }

                // Send heartbeat
                _ = heartbeat_interval.tick() => {
                    if let Err(e) = self.connection.send(PacketType::Heartbeat, &[]).await {
                        error!("Failed to send heartbeat: {}", e);
                        break;
                    }
                }

                // Check timeout
                _ = timeout_interval.tick() => {
                    if self.is_timed_out() {
                        info!("Session timed out");
                        let _ = self.connection.send(PacketType::SessionTimeout, &[]).await;
                        break;
                    }
                }
            }

            if !self.is_running() {
                break;
            }
        }

        self.stop();
        let _ = capture_handle.await;

        info!("Session with {} ended after {}ms", self.addr, self.uptime_ms());

        Ok(())
    }

    /// Handle an incoming packet
    async fn handle_packet(
        &mut self,
        packet_type: PacketType,
        payload: &[u8],
        input: &mut InputController,
    ) -> Result<bool, NetworkError> {
        match packet_type {
            PacketType::SessionEnd => {
                return Ok(false);
            }

            PacketType::Heartbeat => {
                self.connection.send(PacketType::HeartbeatAck, &[]).await?;
            }

            PacketType::HeartbeatAck => {
                debug!("Heartbeat acknowledged");
            }

            PacketType::ScreenRequest => {
                // Force send current frame
                debug!("Screen request received");
            }

            PacketType::MouseMove => {
                if payload.len() >= 2 {
                    let movement = MouseMove {
                        dx: payload[0] as i8,
                        dy: payload[1] as i8,
                    };
                    input.mouse_move(movement);
                    debug!("Mouse move: {:?}", movement);
                }
            }

            PacketType::MouseClick => {
                if let Ok(click) = serde_json::from_slice::<MouseClick>(payload) {
                    input.mouse_click(click);
                    debug!("Mouse click: {:?}", click);
                }
            }

            PacketType::KeyPress => {
                if let Ok(event) = serde_json::from_slice::<KeyEvent>(payload) {
                    // In mouse mode, arrow keys move mouse
                    if input.get_mode() == InputMode::Mouse {
                        input.arrow_to_mouse(event.keycode);
                    } else {
                        input.key_press(event);
                    }
                    debug!("Key press: {:?}", event);
                }
            }

            PacketType::KeyRelease => {
                if let Ok(event) = serde_json::from_slice::<KeyEvent>(payload) {
                    if input.get_mode() == InputMode::Keyboard {
                        input.key_release(event);
                    }
                    debug!("Key release: {:?}", event);
                }
            }

            PacketType::KeyType => {
                // Type a string directly
                if let Ok(text) = std::str::from_utf8(payload) {
                    input.type_string(text);
                    debug!("Key type: {}", text);
                }
            }

            PacketType::ModeSwitch => {
                if let Ok(mode_switch) = serde_json::from_slice::<ModeSwitch>(payload) {
                    input.switch_mode(mode_switch.mode);
                    info!("Mode switched to {:?}", mode_switch.mode);

                    // Acknowledge mode switch
                    let ack = serde_json::to_vec(&ModeSwitch {
                        mode: input.get_mode(),
                    })
                    .unwrap_or_default();
                    self.connection.send(PacketType::ModeAck, &ack).await?;
                } else {
                    // Toggle mode if no payload
                    let new_mode = input.toggle_mode();
                    info!("Mode toggled to {:?}", new_mode);

                    let ack = serde_json::to_vec(&ModeSwitch { mode: new_mode })
                        .unwrap_or_default();
                    self.connection.send(PacketType::ModeAck, &ack).await?;
                }
            }

            _ => {
                warn!("Unhandled packet type: {:?}", packet_type);
            }
        }

        Ok(true)
    }

    /// Send a screen frame
    async fn send_frame(&mut self, frame: CapturedFrame) -> Result<(), NetworkError> {
        let screen_frame = ScreenFrame {
            sequence: frame.sequence,
            timestamp: self.uptime_ms() as u32,
            jpeg_data: frame.jpeg_data,
        };

        let payload = screen_frame.serialize();

        debug!(
            "Sending frame {} ({} bytes)",
            screen_frame.sequence,
            payload.len()
        );

        self.connection.send(PacketType::ScreenFrame, &payload).await
    }

    /// Send session end notification
    pub async fn send_end(&mut self) -> Result<(), NetworkError> {
        self.connection.send(PacketType::SessionEnd, &[]).await
    }
}

/// Session statistics for logging
#[derive(Debug, Clone, Default)]
pub struct SessionStats {
    pub frames_sent: u64,
    pub bytes_sent: u64,
    pub commands_received: u64,
    pub start_time: Option<Instant>,
    pub end_time: Option<Instant>,
}

impl SessionStats {
    pub fn new() -> Self {
        Self {
            start_time: Some(Instant::now()),
            ..Default::default()
        }
    }

    pub fn duration(&self) -> Duration {
        match (self.start_time, self.end_time) {
            (Some(start), Some(end)) => end.duration_since(start),
            (Some(start), None) => start.elapsed(),
            _ => Duration::ZERO,
        }
    }

    pub fn avg_fps(&self) -> f64 {
        let secs = self.duration().as_secs_f64();
        if secs > 0.0 {
            self.frames_sent as f64 / secs
        } else {
            0.0
        }
    }
}

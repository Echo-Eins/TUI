# Network Tab Widget & Function Designs (ASCII)

This document defines the target production UI design for the Network tab.
The design is implementation-oriented and aligned with interactive diagnostics.

-------------------------------------------------------------------------------
01) Full Layout (Primary View)
-------------------------------------------------------------------------------

+-------------------------------------------------------------------------------------------------------+
| Network: UP (2 ifaces) | Default: eth0 via 192.168.1.1 | DNS mode: systemd-resolved | Jobs: 0 running |
+--------------------------+------------------------------------+------------------------------------------------+
| TOOLS                    | PARAMETERS (Editable)              | RESULTS                                        |
|--------------------------|------------------------------------|------------------------------------------------|
| [/] Search: ping         | Tool: Ping+                        | Tabs: [Summary] [Details] [Raw] [Advice] [Hist]|
|                          | Profile: (x) quick ( ) latency     |------------------------------------------------|
| > Resolve                |          ( ) loss                  | Verdict: OK                                    |
|   DNS Explain            | Target: [ 1.1.1.1               ]  | Target: 1.1.1.1                                |
|   Route Inspect          | Mode:   ( ) count   (x) continuous | Loss: 0.0%   Samples: 64                       |
|   NIC Deep Info          | Count:  [ 64 ]                     | RTT: min 8.4 / avg 9.1 / p95 11.7 / p99 13.2   |
|   Connection Lab         | Timeout:[ 2s ]  Interval:[250ms]   | Jitter: 0.7ms                                  |
| > Ping+                  | Deadline:[ 20s ]                   |                                                |
| > Trace+                 | Family: [ auto v ]                 | Sparkline RTT: _-__--_-___--__--_---__-__      |
|   MTU Probe              | Advanced [>]                       |                                                |
|   Port Scan              |------------------------------------| Next Action:                                   |
|   NAT Capability         | [Run] [Cancel] [Repeat] [Export]   | - Stable path                                  |
|   Mapping Test           | Last run: 22:14:32  Duration: 19s  | - No packet loss                               |
|   Export Report          | State: COMPLETED                   |                                                |
+--------------------------+------------------------------------+------------------------------------------------+
| ACTIVITY (stream, capped ring buffer)                                                                     |
| [22:14:13] Job#44 started: Ping+                                                                          |
| [22:14:13] profile=latency target=1.1.1.1 count=64 interval=250ms                                         |
| [22:14:32] completed: loss 0.0%, p95 11.7ms                                                               |
+-----------------------------------------------------------------------------------------------------------+
| Focus: [Tools] [Params] [Actions] [Results] [Activity] | Tab/Shift+Tab switch focus | PgUp/PgDn scroll |
+-----------------------------------------------------------------------------------------------------------+

-------------------------------------------------------------------------------
02) Compact Layout (Narrow Terminal)
-------------------------------------------------------------------------------

+-------------------------------------------------------------------------------------------------------+
| Net: UP | GW 192.168.1.1 | DNS resolved | Job: idle                                                  |
+----------------------------------------------+------------------------------------------------------------+
| TOOL + QUICK PARAMS                           | RESULT / EVENTS                                            |
|-----------------------------------------------|------------------------------------------------------------|
| Tool: Trace+ [v]                              | Summary: PARTIAL                                           |
| Target: [example.org                     ]    | Used protocol: TCP (fallback from ICMP)                    |
| Proto: [icmp v] Fallback:[on] Hops:[20]      | Reached target: no                                         |
| Timeout:[2s] Q:[1] Port:[443] Resolve:[off]  | Blocked indicators: high timeout ratio                     |
| [Run] [Cancel] [Export]                       |                                                            |
|-----------------------------------------------| Hop table (top 6)                                          |
| Recent tools: ping, trace, dns               | 01 192.168.1.1   1.2ms                                     |
|                                               | 02 10.0.0.1      4.7ms                                     |
|                                               | 03 * * *                                                   |
|                                               | 04 * * *                                                   |
|                                               | 05 203.0.113.9   28.1ms !N                                 |
|                                               | 06 * * *                                                   |
+-------------------------------------------------------------------------------------------------------+

-------------------------------------------------------------------------------
03) Tool Navigator Widget
-------------------------------------------------------------------------------

A) Category List mode

+----------------------------------+
| TOOLS                            |
|----------------------------------|
| [/] Search: dns                  |
|                                  |
| DNS                              |
|  > Resolve                       |
|  > DNS Explain                   |
|                                  |
| Routing                          |
|    Route Inspect                 |
|    Trace+                        |
|    MTU Probe                     |
|                                  |
| Interfaces                       |
|    NIC Deep Info                 |
|                                  |
| Traffic                          |
|    Connection Lab                |
|    Ping+                         |
|    Port Scan                     |
|                                  |
| NAT                              |
|    NAT Capability                |
|    Mapping Test                  |
|                                  |
| Reporting                        |
|    Export Report                 |
+----------------------------------+

B) Dropdown mode (Space)

+----------------------------------------------------+
| Select Tool                                        |
|----------------------------------------------------|
| > Ping+           [Traffic]   last: OK  22:14      |
|   Trace+          [Routing]   last: WARN 22:09     |
|   DNS Explain     [DNS]       last: OK  22:06      |
|   Route Inspect   [Routing]   last: OK  22:05      |
|   MTU Probe       [Routing]   last: FAIL 21:52     |
|   Port Scan       [Traffic]   last: OK  21:50      |
|                                                    |
| Enter=Select  Esc=Close  /=Filter                 |
+----------------------------------------------------+

-------------------------------------------------------------------------------
04) Parameter Form Widget (Editable-Only)
-------------------------------------------------------------------------------

+----------------------------------------------------------------------------------+
| PARAMETERS                                                                        |
|----------------------------------------------------------------------------------|
| Tool: Trace+                                                                      |
| Presets: [Quick] [Balanced] [Deep]                                               |
|                                                                                  |
| Target             [ example.org                                       ]         |
| Protocol           [ ICMP v ]                                                    |
| Fallback           [x] Enabled                                                   |
| Max hops           [ 20 ]                                                        |
| Timeout per hop    [ 2s ]                                                        |
| Queries per hop    [ 1 ]                                                         |
| Port (for TCP)     [ 443 ]                                                       |
| Resolve hostnames  [ ]                                                           |
|                                                                                  |
| [Run] [Cancel] [Repeat Last] [Reset] [Export]                                   |
|                                                                                  |
| Validation:                                                                       |
| - Target is required                                                              |
| - hops: 1..64                                                                     |
| - timeout: 1..10s                                                                 |
| - q: 1..5                                                                         |
+----------------------------------------------------------------------------------+

Rule:
- Do not show non-editable "additional params" in this widget.
- Derived values are shown only in result section as "Effective config".

-------------------------------------------------------------------------------
05) Result Workspace Widget (Tab System)
-------------------------------------------------------------------------------

A) Summary Tab

+----------------------------------------------------------------------------------+
| RESULTS > SUMMARY                                                                |
|----------------------------------------------------------------------------------|
| Tool: Trace+              Request: ICMP               Used: TCP (fallback)      |
| Verdict: PARTIAL          Duration: 17.2s             Job ID: #71                |
|                                                                                  |
| Reached target: no                                                                |
| Hops parsed: 20                                                                   |
| Timeout ratio: 0.65                                                               |
| Blocked indicators: High timeout ratio, !N marker                                |
|                                                                                  |
| Effective config:                                                                 |
| target=example.org proto=icmp fallback=on hops=20 timeout=2 q=1 port=443         |
+----------------------------------------------------------------------------------+

B) Details Tab

+----------------------------------------------------------------------------------+
| RESULTS > DETAILS                                                                |
|----------------------------------------------------------------------------------|
| Hop | Host/IP                  | RTTs (ms)           | Resp/Send | Flags         |
|-----+--------------------------+---------------------+-----------+---------------|
|  1  | 192.168.1.1              | 1.2                 | 1/1       |               |
|  2  | 10.0.0.1                 | 4.7                 | 1/1       |               |
|  3  | *                        | -                   | 0/1       | timeout       |
|  4  | *                        | -                   | 0/1       | timeout       |
|  5  | 203.0.113.9              | 28.1                | 1/1       | blocked (!N)  |
| ...                                                                            ... |
+----------------------------------------------------------------------------------+

C) Raw Tab

+----------------------------------------------------------------------------------+
| RESULTS > RAW                                                                    |
|----------------------------------------------------------------------------------|
| stdout:                                                                           |
| traceroute to example.org (93.184.216.34), 20 hops max, 60 byte packets          |
| 1  192.168.1.1  1.2 ms                                                            |
| 2  10.0.0.1  4.7 ms                                                               |
| 3  *                                                                              |
| ...                                                                               |
|----------------------------------------------------------------------------------|
| stderr:                                                                           |
| <empty>                                                                           |
+----------------------------------------------------------------------------------+

D) Advice Tab

+----------------------------------------------------------------------------------+
| RESULTS > ADVICE                                                                 |
|----------------------------------------------------------------------------------|
| Diagnosis:                                                                        |
| - Path is partially filtered for ICMP/UDP probes                                 |
| - TCP trace reached further hops than ICMP                                        |
|                                                                                  |
| Recommended next actions:                                                         |
| 1) Retry with proto=tcp q=2 timeout=3                                             |
| 2) Run MTU Probe to check PMTU black-hole                                         |
| 3) Compare with ping profile=loss deadline=30                                     |
+----------------------------------------------------------------------------------+

E) History Tab

+----------------------------------------------------------------------------------+
| RESULTS > HISTORY                                                                |
|----------------------------------------------------------------------------------|
| Time      | Tool    | Target         | Verdict | Key metrics                      |
|-----------+---------+----------------+---------+----------------------------------|
| 22:14:32  | Ping+   | 1.1.1.1        | OK      | loss 0.0%, p95 11.7ms           |
| 22:09:11  | Trace+  | example.org    | PARTIAL | timeout 0.65, fallback yes      |
| 22:06:01  | DNS Exp | system         | OK      | conflicts 0                     |
+----------------------------------------------------------------------------------+

-------------------------------------------------------------------------------
06) Activity Widget (Separate from Results)
-------------------------------------------------------------------------------

+----------------------------------------------------------------------------------+
| ACTIVITY                                                                         |
|----------------------------------------------------------------------------------|
| [22:14:13.012] INFO  Job#44 started: Ping+                                       |
| [22:14:13.019] INFO  target=1.1.1.1 profile=latency count=64 interval=250ms      |
| [22:14:15.431] INFO  progress: 8/64                                               |
| [22:14:24.117] INFO  progress: 40/64                                              |
| [22:14:32.205] OK    completed: loss 0.0%, p95 11.7ms                            |
|                                                                                  |
| [K] Clear Activity   [Ctrl+S] Save Snapshot                                      |
+----------------------------------------------------------------------------------+

Clarification:
- K clears only this stream.
- Completion state is controlled by job lifecycle, not by K.

-------------------------------------------------------------------------------
07) Per-Function ASCII Mini-Designs (All 12 tools)
-------------------------------------------------------------------------------

01) Resolve

+--------------------------------------------------+
| Resolve                                           |
| Target [ example.org ]  Family [auto v]          |
| [Run]                                            |
|--------------------------------------------------|
| Host: example.org                                |
| A: 93.184.216.34                                 |
| AAAA: 2606:2800:220:1:248:1893:25c8:1946         |
| Source: systemd-resolved                         |
+--------------------------------------------------+

02) DNS Explain

+--------------------------------------------------+
| DNS Explain                                       |
| Include gateways [x]                              |
| [Run]                                            |
|--------------------------------------------------|
| Resolver mode: systemd-resolved-stub             |
| resolv.conf -> /run/systemd/resolve/stub-resolv.conf |
| NM dns mode: default                             |
| Conflicts: none                                  |
| Servers: 1.1.1.1, 8.8.8.8                        |
+--------------------------------------------------+

03) Route Inspect

+--------------------------------------------------+
| Route Inspect                                     |
| Target [ 1.1.1.1 ] Include policy [x]            |
| [Run]                                            |
|--------------------------------------------------|
| default via 192.168.1.1 dev eth0 metric 100      |
| rule prio 32766 lookup main                       |
| egress to 1.1.1.1: dev eth0 src 192.168.1.105     |
+--------------------------------------------------+

04) NIC Deep Info

+--------------------------------------------------+
| NIC Deep Info                                     |
| Interface [ all v ] Stats [x] WiFi [x]           |
| [Run]                                            |
|--------------------------------------------------|
| eth0: Intel I225-V driver=igc fw=1.57            |
| speed=2500Mb duplex=full mtu=1500                 |
| offloads: tso on, gso on, gro on                  |
| drops(rx/tx)=0/0 errors(rx/tx)=0/0                |
+--------------------------------------------------+

05) Connection Lab

+--------------------------------------------------+
| Connection Lab                                    |
| Proto [tcp v] State [estab v] Limit [200]        |
| Extended metrics [x]                              |
| [Run]                                            |
|--------------------------------------------------|
| PID  Proc      Local              Remote          |
| 5321 chrome    192.168.1.105:54321 142.250...:443 |
| notes: retransmits=1 bytes_sent=2MB bytes_recv=8MB|
+--------------------------------------------------+

06) Ping+

+--------------------------------------------------+
| Ping+                                             |
| Target [ 1.1.1.1 ] Profile [latency v]           |
| Continuous [x] Deadline [20s]                     |
| Count [64] Timeout [2s] Interval [250ms]         |
| [Run]                                            |
|--------------------------------------------------|
| tx=64 rx=64 loss=0.0%                             |
| min/avg/max=8.4/9.1/13.8 ms                       |
| p50/p95/p99=9.0/11.7/13.2 ms                      |
| jitter=0.7 ms                                     |
+--------------------------------------------------+

07) Trace+

+--------------------------------------------------+
| Trace+                                            |
| Target [ example.org ] Proto [icmp v]            |
| Fallback [x] Hops [20] Timeout [2s] Q [1]        |
| Port [443] Resolve names [ ]                     |
| [Run]                                            |
|--------------------------------------------------|
| Requested: ICMP  Used: TCP (fallback)            |
| reached=no timeout_ratio=0.65                     |
| blocked indicators: !N, high timeout ratio       |
+--------------------------------------------------+

08) MTU Probe

+--------------------------------------------------+
| MTU Probe                                         |
| Target [ 1.1.1.1 ]                                |
| [Run]                                            |
|--------------------------------------------------|
| path_mtu=1472                                     |
| eth0 mtu=1500                                     |
| advice: set MSS clamp 1432 if VPN tunnel present |
+--------------------------------------------------+

09) Port Scan

+--------------------------------------------------+
| Port Scan                                         |
| Target [ example.org ] Profile [web v]           |
| Ports [ 80,443,8080 ] Timeout [450ms]            |
| [Run]                                            |
|--------------------------------------------------|
| open: 80,443                                      |
| closed: 8080                                      |
| filtered: none                                    |
+--------------------------------------------------+

10) NAT Capability

+--------------------------------------------------+
| NAT Capability                                    |
| Timeout [8s]                                      |
| [Run]                                            |
|--------------------------------------------------|
| UPnP: supported                                   |
| NAT-PMP: unavailable                              |
| PCP: missing dependency                           |
| External IP: 203.0.113.5                          |
+--------------------------------------------------+

11) Mapping Test

+--------------------------------------------------+
| Mapping Test                                      |
| Protocol [tcp v] In [8080] Out [8080] TTL [120]  |
| Confirm [ ] I understand this is active test      |
| [Run]                                            |
|--------------------------------------------------|
| created=true listed=true removed=true             |
| cleanup_status=clean                              |
+--------------------------------------------------+

12) Export Report

+--------------------------------------------------+
| Export Report                                     |
| Format [json v] Max entries [64]                 |
| [Run]                                            |
|--------------------------------------------------|
| path: ./reports/network_2026-02-27_22-14-32.json |
| size: 84 KB                                       |
| sha256: 9c2a...                                   |
+--------------------------------------------------+

-------------------------------------------------------------------------------
08) Focus Navigation Contract
-------------------------------------------------------------------------------

+----------------------------------------------------------------------------------+
| FOCUS MAP                                                                         |
|----------------------------------------------------------------------------------|
| [Tools] -> [Params] -> [Actions] -> [Results] -> [Activity] -> [Tools]           |
|                                                                                  |
| Tab / Shift+Tab : change panel focus                                             |
| Up/Down          : move in focused list/table                                     |
| Left/Right       : switch result sub-tab                                          |
| PgUp/PgDn        : scroll content                                                  |
| Home/End         : jump to top/bottom                                              |
| Enter            : activate selected action                                        |
| Esc              : close dropdown/search modal                                     |
+----------------------------------------------------------------------------------+

-------------------------------------------------------------------------------
09) Lifecycle Semantics (No manual stop for self-terminating jobs)
-------------------------------------------------------------------------------

+----------------------------------------------------------------------------------+
| JOB STATE                                                                         |
|----------------------------------------------------------------------------------|
| Idle -> Running -> Completed | Failed | Cancelled -> Archived                     |
|                                                                                  |
| Completed jobs auto-free running slot and spinner.                                |
| K only clears Activity logs.                                                      |
| X only cancels currently running job.                                             |
+----------------------------------------------------------------------------------+

-------------------------------------------------------------------------------
10) Production UX Rules (Network tab)
-------------------------------------------------------------------------------

1. Any shown field in Parameters must be editable.
2. Any non-editable value must move to "Effective config" in Results.
3. Results must never be log-only; Summary is mandatory.
4. All tools must support cancel/timeout/export in a consistent location.
5. Permission-limited results must include explicit explanation and workaround.
6. Navigator must support scroll, search and dropdown selection.
7. One-shot diagnostics must end cleanly without manual clear key usage.

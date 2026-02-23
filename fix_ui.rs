use std::fs;
use std::path::Path;

fn main() {
    let broken = ["â†‘â†“", "â”‚", "â† â†’"];
    let fixed = ["↑↓", "│", "←→"];
    
    // We will just define the known files manually since glob isn't in default crate
    let files = vec![
        "src/ui/mod.rs",
        "src/ui/tabs/cpu.rs",
        "src/ui/tabs/gpu.rs",
        "src/ui/tabs/ram.rs",
        "src/ui/tabs/processes.rs",
        "src/ui/tabs/services.rs"
    ];

    for path_str in files {
        let path = Path::new(path_str);
        if !path.exists() { continue; }
        
        let content = fs::read_to_string(&path).unwrap();
        let mut new_content = content.clone();
        
        for (b, f) in broken.iter().zip(fixed.iter()) {
            new_content = new_content.replace(*b, *f);
        }
        
        if new_content != content {
            fs::write(&path, new_content).unwrap();
            println!("Fixed {:?}", path);
        }
    }
}

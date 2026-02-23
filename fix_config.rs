use std::fs;

fn main() {
    let path = "src/app/default_config.toml";
    let mut content = fs::read_to_string(path).unwrap();
    
    // Add "console" to enabled tabs if missing
    if !content.contains("\"console\"") {
        content = content.replace(
            "enabled = [\"cpu\", \"gpu\", \"ram\", \"disk\", \"network\", \"ollama\", \"processes\", \"services\", \"disk_analyzer\", \"settings\"]",
            "enabled = [\"cpu\", \"gpu\", \"ram\", \"disk\", \"network\", \"ollama\", \"processes\", \"services\", \"console\", \"disk_analyzer\", \"settings\"]"
        );
    }
    
    // Insert "console = 9" and push the others down
    if !content.contains("console = \"9\"") {
        content = content.replace(
            "services = \"8\"\ndisk_analyzer = \"9\"\nsettings = \"0\"",
            "services = \"8\"\nconsole = \"9\"\ndisk_analyzer = \"c\"\nsettings = \"0\""
        );
    }
    
    fs::write(path, content).unwrap();
    println!("Updated default config");
}

use notify_rust::Notification;

fn main() {
    println!("Testing direct notification...");
    match Notification::new()
        .summary("Test Direct")
        .body("Notification de test direct")
        .appname("Test")
        .icon("dialog-information")
        .timeout(5000)
        .show()
    {
        Ok(handle) => println!("✓ Notification sent successfully: {:?}", handle),
        Err(e) => eprintln!("✗ Error: {}", e),
    }
    
    // Wait a bit to ensure notification is shown
    std::thread::sleep(std::time::Duration::from_secs(6));
}

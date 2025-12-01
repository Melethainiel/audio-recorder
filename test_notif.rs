use notify_rust::Notification;

fn main() {
    println!("Testing notification...");
    match Notification::new()
        .summary("Test Audio Recorder")
        .body("Ceci est un test de notification")
        .appname("Audio Recorder")
        .icon("audio-input-microphone")
        .timeout(5000)
        .show()
    {
        Ok(_) => println!("Notification sent successfully!"),
        Err(e) => eprintln!("Error sending notification: {}", e),
    }
}

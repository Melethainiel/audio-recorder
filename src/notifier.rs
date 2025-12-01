use std::sync::mpsc::{channel, Sender};
use std::thread;

pub struct Notifier {
    sender: Sender<(String, String)>,
}

impl Notifier {
    pub fn new() -> Self {
        let (sender, receiver) = channel::<(String, String)>();
        
        // Spawn a dedicated thread for notifications
        thread::spawn(move || {
            while let Ok((title, body)) = receiver.recv() {
                Self::send_notification(&title, &body);
            }
        });
        
        Notifier { sender }
    }
    
    pub fn notify(&self, title: &str, body: &str) {
        let _ = self.sender.send((title.to_string(), body.to_string()));
    }
    
    fn send_notification(title: &str, body: &str) {
        use notify_rust::Notification;
        
        match Notification::new()
            .summary(title)
            .body(body)
            .appname("Audio Recorder")
            .icon("audio-input-microphone")
            .timeout(5000)
            .show()
        {
            Ok(_) => println!("✓ Notification sent: {} - {}", title, body),
            Err(e) => eprintln!("✗ Failed to send notification: {}", e),
        }
    }
}

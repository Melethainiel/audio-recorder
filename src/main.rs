use gtk4::prelude::*;
use gtk4::{glib, Application, ApplicationWindow, Button, Orientation, DrawingArea};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use chrono::Local;
use vorbis_rs::VorbisEncoderBuilder;
use std::fs::File;
use std::rc::Rc;
use std::cell::RefCell;
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

const APP_ID: &str = "com.audio.recorder";

// Global window reference for tray icon to toggle
static WINDOW_VISIBLE: Mutex<Option<Arc<Mutex<bool>>>> = Mutex::new(None);

#[derive(Serialize, Deserialize, Clone)]
struct Config {
    selected_mic_index: usize,
    selected_loopback_index: Option<usize>,
    mic_gain: f32,
    save_directory: Option<String>,
    n8n_endpoint: Option<String>,
    n8n_enabled: bool,
    save_locally: bool,
}

impl Config {
    fn load() -> Option<Self> {
        let config_path = dirs::config_dir()?.join("audio-recorder").join("config.json");
        let mut file = File::open(config_path).ok()?;
        let mut contents = String::new();
        file.read_to_string(&mut contents).ok()?;
        serde_json::from_str(&contents).ok()
    }
    
    fn save(&self) -> std::io::Result<()> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "Config dir not found"))?
            .join("audio-recorder");
        
        std::fs::create_dir_all(&config_dir)?;
        let config_path = config_dir.join("config.json");
        let json = serde_json::to_string_pretty(self)?;
        let mut file = File::create(config_path)?;
        file.write_all(json.as_bytes())?;
        Ok(())
    }
}

fn main() -> glib::ExitCode {
    // List available audio devices at startup
    let host = cpal::default_host();
    println!("=== Available input devices ===");
    if let Ok(devices) = host.input_devices() {
        for device in devices {
            if let Ok(name) = device.name() {
                let is_monitor = if name.contains(".monitor") { " [MONITOR]" } else { "" };
                println!("  - {}{}", name, is_monitor);
            }
        }
    }
    println!("===============================");

    let app = Application::builder()
        .application_id(APP_ID)
        .build();

    app.connect_activate(build_ui);
    
    // Start tray icon in background
    std::thread::spawn(|| {
        start_tray_icon();
    });

    app.run()
}

fn start_tray_icon() {
    use ksni::TrayService;
    
    struct RecorderTray;
    
    impl ksni::Tray for RecorderTray {
        fn id(&self) -> String {
            "audio-recorder".to_string()
        }
        
        fn title(&self) -> String {
            "Audio Recorder".to_string()
        }
        
        fn icon_name(&self) -> String {
            "audio-input-microphone".to_string()
        }
        
        fn activate(&mut self, _x: i32, _y: i32) {
            // Called on activation (click behavior varies by DE)
            toggle_window_visibility();
        }
        
        fn secondary_activate(&mut self, _x: i32, _y: i32) {
            // Called on middle click
            toggle_window_visibility();
        }
        
        fn menu(&self) -> Vec<ksni::MenuItem<Self>> {
            use ksni::menu::*;
            vec![
                StandardItem {
                    label: "Show/Hide".to_string(),
                    activate: Box::new(|_| {
                        toggle_window_visibility();
                    }),
                    ..Default::default()
                }.into(),
                StandardItem {
                    label: "Quit".to_string(),
                    activate: Box::new(|_| {
                        std::process::exit(0);
                    }),
                    ..Default::default()
                }.into(),
            ]
        }
    }
    
    fn toggle_window_visibility() {
        if let Ok(guard) = WINDOW_VISIBLE.lock() {
            if let Some(visible_ref) = guard.as_ref() {
                if let Ok(mut visible) = visible_ref.lock() {
                    *visible = !*visible;
                    println!("Window visibility toggled to: {}", *visible);
                }
            }
        }
    }
    
    let service = TrayService::new(RecorderTray);
    let _handle = service.spawn();
    println!("Tray icon started!");
    // Keep the service alive
    loop {
        std::thread::sleep(Duration::from_secs(1));
    }
}

#[derive(Clone)]
struct AudioSource {
    name: String,
    display_name: String,
    is_monitor: bool,
}

struct RecorderState {
    recording: bool,
    paused: bool,
    start_time: Option<Instant>,
    elapsed: Duration,
    input_stream: Option<cpal::Stream>,
    output_stream: Option<cpal::Stream>,
    input_level: Arc<Mutex<f32>>,
    output_level: Arc<Mutex<f32>>,
    input_samples: Arc<Mutex<Vec<f32>>>,
    output_samples: Arc<Mutex<Vec<f32>>>,
    waveform_history: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    channels: u16,
    available_sources: Vec<AudioSource>,
    selected_mic_index: usize,
    selected_loopback_index: Option<usize>,
    mic_gain: Arc<Mutex<f32>>,
    save_directory: Arc<Mutex<Option<String>>>,
    n8n_endpoint: Arc<Mutex<Option<String>>>,
    n8n_enabled: Arc<Mutex<bool>>,
    save_locally: Arc<Mutex<bool>>,
}

impl RecorderState {
    fn new() -> Self {
        let available_sources = Self::get_available_sources();
        
        // Try to load config, otherwise use defaults
        let config = Config::load();
        
        let selected_mic_index = config
            .as_ref()
            .map(|c| c.selected_mic_index)
            .filter(|&idx| idx < available_sources.len() && !available_sources[idx].is_monitor)
            .or_else(|| {
                available_sources
                    .iter()
                    .position(|s| !s.is_monitor && (s.name == "pulse" || s.name == "pipewire"))
                    .or_else(|| available_sources.iter().position(|s| !s.is_monitor))
            })
            .unwrap_or(0);
        
        let selected_loopback_index = config
            .as_ref()
            .and_then(|c| c.selected_loopback_index)
            .filter(|&idx| idx < available_sources.len() && available_sources[idx].is_monitor)
            .or_else(|| available_sources.iter().position(|s| s.is_monitor));
        
        let mic_gain = config
            .as_ref()
            .map(|c| c.mic_gain)
            .unwrap_or(1.0);
        
        let save_directory = config
            .as_ref()
            .and_then(|c| c.save_directory.clone());
        
        let n8n_endpoint = config
            .as_ref()
            .and_then(|c| c.n8n_endpoint.clone());
        
        let n8n_enabled = config
            .as_ref()
            .map(|c| c.n8n_enabled)
            .unwrap_or(false);
        
        let save_locally = config
            .as_ref()
            .map(|c| c.save_locally)
            .unwrap_or(true);
        
        Self {
            recording: false,
            paused: false,
            start_time: None,
            elapsed: Duration::default(),
            input_stream: None,
            output_stream: None,
            input_level: Arc::new(Mutex::new(0.0)),
            output_level: Arc::new(Mutex::new(0.0)),
            input_samples: Arc::new(Mutex::new(Vec::new())),
            output_samples: Arc::new(Mutex::new(Vec::new())),
            waveform_history: Arc::new(Mutex::new(vec![0.0; 60])),
            sample_rate: 48000,
            channels: 1,
            available_sources,
            selected_mic_index,
            selected_loopback_index,
            mic_gain: Arc::new(Mutex::new(mic_gain)),
            save_directory: Arc::new(Mutex::new(save_directory)),
            n8n_endpoint: Arc::new(Mutex::new(n8n_endpoint)),
            n8n_enabled: Arc::new(Mutex::new(n8n_enabled)),
            save_locally: Arc::new(Mutex::new(save_locally)),
        }
    }
    
    fn save_config(&self) {
        let config = Config {
            selected_mic_index: self.selected_mic_index,
            selected_loopback_index: self.selected_loopback_index,
            mic_gain: *self.mic_gain.lock().unwrap(),
            save_directory: self.save_directory.lock().unwrap().clone(),
            n8n_endpoint: self.n8n_endpoint.lock().unwrap().clone(),
            n8n_enabled: *self.n8n_enabled.lock().unwrap(),
            save_locally: *self.save_locally.lock().unwrap(),
        };
        
        if let Err(e) = config.save() {
            eprintln!("Failed to save config: {}", e);
        } else {
            println!("Config saved successfully");
        }
    }
    
    fn get_available_sources() -> Vec<AudioSource> {
        let mut sources = Vec::new();
        
        // Get detailed source info with descriptions
        if let Ok(output) = std::process::Command::new("pactl")
            .args(["list", "sources"])
            .output()
        {
            if output.status.success() {
                if let Ok(stdout) = String::from_utf8(output.stdout) {
                    let mut current_name = String::new();
                    
                    for line in stdout.lines() {
                        let line = line.trim();
                        
                        if line.starts_with("Name: ") {
                            current_name = line.strip_prefix("Name: ").unwrap_or("").to_string();
                        } else if line.starts_with("Description: ") {
                            let current_desc = line.strip_prefix("Description: ").unwrap_or("").to_string();
                            
                            if !current_name.is_empty() && !current_desc.is_empty() {
                                let is_monitor = current_name.contains(".monitor");
                                
                                sources.push(AudioSource {
                                    name: current_name.clone(),
                                    display_name: current_desc,
                                    is_monitor,
                                });
                                
                                current_name.clear();
                            }
                        }
                    }
                }
            }
        }
        
        sources
    }
    
    fn start_recording(&mut self) {
        let host = cpal::default_host();
        
        self.start_time = Some(Instant::now());
        self.elapsed = Duration::default();
        
        let mic_source_name = self.available_sources
            .get(self.selected_mic_index)
            .map(|s| s.name.clone());
        
        if let Some(mic_name) = mic_source_name {
            println!("Using microphone: {}", mic_name);
            
            unsafe { std::env::set_var("PULSE_SOURCE", &mic_name) };
            
            let pulse_device = host.input_devices()
                .ok()
                .and_then(|mut devices| {
                    devices.find(|d| d.name().map(|n| n == "pulse").unwrap_or(false))
                });
            
            if let Some(input_device) = pulse_device {
                if let Ok(input_config) = input_device.default_input_config() {
                    self.sample_rate = input_config.sample_rate().0;
                    self.channels = input_config.channels();
                    
                    let input_level = Arc::clone(&self.input_level);
                    let input_samples = Arc::clone(&self.input_samples);
                    let waveform_history = Arc::clone(&self.waveform_history);
                    let output_level = Arc::clone(&self.output_level);
                    let mic_gain = Arc::clone(&self.mic_gain);
                    
                    if let Ok(mic_stream) = input_device.build_input_stream(
                        &input_config.into(),
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            let gain = *mic_gain.lock().unwrap();
                            let gained_data: Vec<f32> = data.iter()
                                .map(|&s| (s * gain).clamp(-1.0, 1.0))
                                .collect();
                            
                            let sum: f32 = gained_data.iter().map(|&s| s * s).sum();
                            let rms = (sum / gained_data.len() as f32).sqrt();
                            let mic_level = (rms * 5.0).min(1.0);
                            *input_level.lock().unwrap() = mic_level;
                            
                            let loopback_level = *output_level.lock().unwrap();
                            let combined_level = (mic_level + loopback_level).min(1.0);
                            
                            let mut history = waveform_history.lock().unwrap();
                            history.remove(0);
                            history.push(combined_level);
                            
                            input_samples.lock().unwrap().extend_from_slice(&gained_data);
                        },
                        |err| eprintln!("Mic error: {}", err),
                        None,
                    ) {
                        mic_stream.play().unwrap();
                        self.input_stream = Some(mic_stream);
                    }
                }
            }
            
            unsafe { std::env::remove_var("PULSE_SOURCE") };
        }
        
        // System audio loopback
        let loopback_source_name = self.selected_loopback_index
            .and_then(|i| self.available_sources.get(i))
            .map(|s| s.name.clone());
        
        if let Some(loopback_name) = loopback_source_name {
            println!("Using loopback: {}", loopback_name);
            
            unsafe { std::env::set_var("PULSE_SOURCE", &loopback_name) };
            
            let pulse_device = host.input_devices()
                .ok()
                .and_then(|mut devices| {
                    devices.find(|d| d.name().map(|n| n == "pulse").unwrap_or(false))
                });
            
            if let Some(loopback_device) = pulse_device {
                if let Ok(loopback_config) = loopback_device.default_input_config() {
                    let output_level = Arc::clone(&self.output_level);
                    let output_samples = Arc::clone(&self.output_samples);
                    let target_sample_rate = self.sample_rate;
                    let source_sample_rate = loopback_config.sample_rate().0;
                    let source_channels = loopback_config.channels();
                    
                    if let Ok(loopback_stream) = loopback_device.build_input_stream(
                        &loopback_config.into(),
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            let sum: f32 = data.iter().map(|&s| s * s).sum();
                            let rms = (sum / data.len() as f32).sqrt();
                            *output_level.lock().unwrap() = (rms * 5.0).min(1.0);
                            
                            let mono: Vec<f32> = data
                                .chunks(source_channels as usize)
                                .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
                                .collect();
                            
                            let resampled: Vec<f32> = if source_sample_rate != target_sample_rate {
                                let ratio = target_sample_rate as f32 / source_sample_rate as f32;
                                let new_len = (mono.len() as f32 * ratio) as usize;
                                (0..new_len)
                                    .map(|i| {
                                        let src_idx = (i as f32 / ratio) as usize;
                                        mono.get(src_idx).copied().unwrap_or(0.0)
                                    })
                                    .collect()
                            } else {
                                mono
                            };
                            
                            output_samples.lock().unwrap().extend_from_slice(&resampled);
                        },
                        |err| eprintln!("Loopback error: {}", err),
                        None,
                    ) {
                        loopback_stream.play().unwrap();
                        self.output_stream = Some(loopback_stream);
                    }
                }
            }
            
            unsafe { std::env::remove_var("PULSE_SOURCE") };
        }
        
        self.recording = true;
        println!("Recording started");
    }
    
    fn stop_recording(&mut self) {
        if let Some(stream) = self.input_stream.take() {
            drop(stream);
        }
        if let Some(stream) = self.output_stream.take() {
            drop(stream);
        }
        
        let mic_samples: Vec<f32> = {
            let mut samples = self.input_samples.lock().unwrap();
            let data = samples.clone();
            samples.clear();
            data
        };
        
        let system_samples: Vec<f32> = {
            let mut samples = self.output_samples.lock().unwrap();
            let data = samples.clone();
            samples.clear();
            data
        };
        
        let mic_mono: Vec<f32> = if self.channels == 1 {
            mic_samples
        } else {
            mic_samples
                .chunks(self.channels as usize)
                .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
                .collect()
        };
        
        let max_len = mic_mono.len().max(system_samples.len());
        let mixed_samples: Vec<f32> = (0..max_len)
            .map(|i| {
                let mic = mic_mono.get(i).copied().unwrap_or(0.0);
                let sys = system_samples.get(i).copied().unwrap_or(0.0);
                ((mic + sys) * 0.7).clamp(-1.0, 1.0)
            })
            .collect();
        
        if !mixed_samples.is_empty() {
            let timestamp = Local::now().format("%Y%m%d_%H%M%S");
            let filename = format!("recording_{}.ogg", timestamp);
            
            // Get save directory from config or use current directory
            let save_dir = self.save_directory.lock().unwrap().clone();
            let file_path = if let Some(dir) = save_dir {
                // Create directory if it doesn't exist
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    eprintln!("Failed to create directory {}: {}", dir, e);
                    filename.clone() // Fallback to current directory
                } else {
                    format!("{}/{}", dir.trim_end_matches('/'), filename)
                }
            } else {
                filename.clone()
            };
            
            if let Ok(mut file) = File::create(&file_path) {
                use std::num::NonZero;
                
                let sample_rate = NonZero::new(self.sample_rate).unwrap();
                let channels = NonZero::new(1u8).unwrap();
                
                if let Ok(mut encoder_builder) = VorbisEncoderBuilder::new(
                    sample_rate,
                    channels,
                    &mut file
                ) {
                    if let Ok(mut encoder) = encoder_builder.build() {
                        let chunk_size = self.sample_rate as usize;
                        let num_samples = mixed_samples.len();
                        
                        for start in (0..num_samples).step_by(chunk_size) {
                            let end = (start + chunk_size).min(num_samples);
                            let chunk = vec![mixed_samples[start..end].to_vec()];
                            encoder.encode_audio_block(chunk).ok();
                        }
                        
                        if let Err(e) = encoder.finish() {
                            eprintln!("Error finishing encoder: {:?}", e);
                        } else {
                            println!("Saved: {}", file_path);
                            
                            // Upload to N8N if enabled
                            let n8n_enabled = *self.n8n_enabled.lock().unwrap();
                            if n8n_enabled {
                                if let Some(endpoint) = self.n8n_endpoint.lock().unwrap().clone() {
                                    let save_locally = *self.save_locally.lock().unwrap();
                                    let path_for_upload = file_path.clone();
                                    
                                    // Spawn async upload task
                                    glib::spawn_future_local(async move {
                                        match upload_to_n8n(&path_for_upload, &endpoint).await {
                                            Ok(_) => {
                                                println!("Upload to N8N succeeded");
                                                show_notification("Upload réussi", "Le fichier a été envoyé à N8N");
                                                
                                                // Delete local file if not configured to keep it
                                                if !save_locally {
                                                    if let Err(e) = std::fs::remove_file(&path_for_upload) {
                                                        eprintln!("Failed to delete local file: {}", e);
                                                    } else {
                                                        println!("Local file deleted (save_locally=false)");
                                                    }
                                                }
                                            }
                                            Err(e) => {
                                                eprintln!("Upload to N8N failed: {}", e);
                                                show_notification("Échec de l'upload", &format!("Erreur: {}", e));
                                            }
                                        }
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
        
        self.recording = false;
        self.paused = false;
        self.elapsed = Duration::default();
        *self.input_level.lock().unwrap() = 0.0;
        *self.output_level.lock().unwrap() = 0.0;
        
        let mut history = self.waveform_history.lock().unwrap();
        history.iter_mut().for_each(|v| *v = 0.0);
    }
}

async fn upload_to_n8n(file_path: &str, endpoint: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Starting upload to N8N: {} -> {}", file_path, endpoint);
    
    // Read file synchronously (we're in a glib async context, not tokio)
    let file_content = std::fs::read(file_path)?;
    let filename = std::path::Path::new(file_path)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("recording.ogg")
        .to_string();
    
    // Create multipart form for blocking client
    let file_part = reqwest::blocking::multipart::Part::bytes(file_content)
        .file_name(filename.clone())
        .mime_str("audio/ogg")?;
    
    let form = reqwest::blocking::multipart::Form::new()
        .part("file", file_part)
        .text("filename", filename)
        .text("timestamp", Local::now().to_rfc3339());
    
    // Send request with 30s timeout (using blocking client in async context)
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    
    let response = client
        .post(endpoint)
        .multipart(form)
        .send()?;
    
    if response.status().is_success() {
        Ok(())
    } else {
        Err(format!("HTTP {}: {}", response.status(), response.text()?).into())
    }
}

fn show_notification(title: &str, body: &str) {
    use notify_rust::Notification;
    
    // Use native Linux notifications
    let _ = Notification::new()
        .summary(title)
        .body(body)
        .appname("Audio Recorder")
        .icon("audio-input-microphone")
        .timeout(5000) // 5 seconds
        .show();
}

fn build_ui(app: &Application) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Audio Recorder")
        .default_width(380)
        .default_height(130)
        .resizable(false)
        .decorated(false) // No window borders (popup style)
        .build();
    
    // Note: On Wayland/GNOME, we cannot force window position
    // The user can manually position the window where they want (top-right recommended)
    // GNOME will remember the position for future sessions

    // Apply CSS for rounded corners and shadow
    let css_provider = gtk4::CssProvider::new();
    css_provider.load_from_data("
        window {
            background-color: white;
            border-radius: 12px;
        }
        button {
            min-width: 44px;
            min-height: 44px;
            border-radius: 22px;
            font-size: 18px;
        }
        .close-button {
            min-width: 32px;
            min-height: 32px;
            border-radius: 16px;
            font-size: 24px;
            font-weight: bold;
            padding: 0;
            background: transparent;
            border: none;
            color: #666;
        }
        .close-button:hover {
            background: #f0f0f0;
            color: #ef4444;
        }
        .timer-label {
            color: #1f2937;
            font-weight: bold;
        }
        /* Settings dialog styles */
        .settings-combo {
            min-height: 30px;
            font-size: 12px;
        }
        .settings-combo button {
            min-height: 30px;
            font-size: 12px;
            padding: 2px 6px;
        }
        .settings-label {
            font-size: 11px;
            color: #1f2937;
        }
        .settings-scale {
            min-height: 24px;
        }
        .settings-scale slider {
            min-height: 14px;
            min-width: 14px;
        }
        .settings-button {
            min-height: 30px;
            min-width: 65px;
            font-size: 12px;
            padding: 4px 12px;
        }
        .settings-entry {
            min-height: 30px;
            font-size: 12px;
            padding: 4px 8px;
        }
        window.dialog headerbar {
            min-height: 38px;
        }
    ");
    
    gtk4::style_context_add_provider_for_display(
        &gtk4::prelude::WidgetExt::display(&window),
        &css_provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    // Add drag gesture to move window (since we have no titlebar)
    // Add it to a container that will receive the gesture
    let gesture = gtk4::GestureDrag::new();
    let window_clone = window.clone();
    gesture.connect_drag_begin(move |gesture, x, y| {
        if let Some(device) = gesture.device() {
            if let Some(surface) = window_clone.surface() {
                use gtk4::gdk;
                if let Ok(toplevel) = surface.downcast::<gdk::Toplevel>() {
                    toplevel.begin_move(&device, 0, x, y, gdk::CURRENT_TIME);
                }
            }
        }
    });

    let state = Rc::new(RefCell::new(RecorderState::new()));
    
    // Setup window visibility control (needed early for close button)
    let visible = Arc::new(Mutex::new(false)); // Start hidden
    *WINDOW_VISIBLE.lock().unwrap() = Some(Arc::clone(&visible));

    // Main container
    let vbox = gtk4::Box::new(Orientation::Vertical, 0);
    
    // Custom title bar with close button
    let titlebar = gtk4::Box::new(Orientation::Horizontal, 0);
    titlebar.set_margin_top(8);
    titlebar.set_margin_bottom(4);
    titlebar.set_margin_start(12);
    titlebar.set_margin_end(8);
    
    // Empty space for dragging (left side)
    let drag_area = gtk4::Box::new(Orientation::Horizontal, 0);
    drag_area.set_hexpand(true);
    
    // Add the drag gesture ONLY to drag_area, not whole titlebar
    drag_area.add_controller(gesture);
    
    titlebar.append(&drag_area);
    
    // Settings button
    let settings_button = Button::with_label("⚙");
    settings_button.add_css_class("close-button");
    settings_button.set_tooltip_text(Some("Settings"));
    let state_for_settings = Rc::clone(&state);
    let window_for_settings = window.clone();
    settings_button.connect_clicked(move |_| {
        show_settings_dialog(&window_for_settings, &state_for_settings);
    });
    titlebar.append(&settings_button);
    
    // Close button (right side)
    let close_button = Button::with_label("×");
    close_button.add_css_class("close-button");
    close_button.set_tooltip_text(Some("Hide window"));
    let visible_for_close = Arc::clone(&visible);
    close_button.connect_clicked(move |_| {
        println!("Close button clicked!");
        if let Ok(mut v) = visible_for_close.try_lock() {
            println!("Setting visible to false");
            *v = false;
        } else {
            println!("Failed to lock visible");
        }
    });
    titlebar.append(&close_button);
    
    vbox.append(&titlebar);
    
    // Content container
    let content = gtk4::Box::new(Orientation::Vertical, 10);
    content.set_margin_top(8);
    content.set_margin_bottom(16);
    content.set_margin_start(16);
    content.set_margin_end(16);

    // Waveform drawing area
    let drawing_area = DrawingArea::new();
    drawing_area.set_content_width(348);
    drawing_area.set_content_height(50);
    
    let state_clone = Rc::clone(&state);
    drawing_area.set_draw_func(move |_area, cr, width, height| {
        let state = state_clone.borrow();
        
        // Background
            let _ = cr.set_source_rgb(0.95, 0.96, 0.96);
        let _ = cr.rectangle(0.0, 0.0, width as f64, height as f64);
        let _ = cr.fill();
        
        // Bars
        let waveform_data = state.waveform_history.lock().unwrap();
        let num_bars = 60;
        let bar_width = 3.0;
        let spacing = (width as f64 - (num_bars as f64 * bar_width)) / (num_bars as f64 + 1.0);
        
        for i in 0..num_bars {
            let x = spacing + (i as f64 * (bar_width + spacing));
            let level = if i < waveform_data.len() {
                waveform_data[i] as f64
            } else {
                0.0
            };
            
            let bar_height = (level * height as f64 * 0.8).max(4.0).min(height as f64 * 0.9);
            let y = (height as f64 - bar_height) / 2.0;
            
            if state.recording && level > 0.05 {
                cr.set_source_rgb(0.13, 0.77, 0.37); // Green
            } else {
                cr.set_source_rgb(0.82, 0.84, 0.86); // Gray
            }
            
            let _ = cr.rectangle(x, y, bar_width, bar_height);
            let _ = cr.fill();
        }
    });
    
    content.append(&drawing_area);

    // Controls
    let controls = gtk4::Box::new(Orientation::Horizontal, 20);
    controls.set_halign(gtk4::Align::Center);

    // Pause button
    let pause_button = Button::with_label("⏸");
    let state_clone = Rc::clone(&state);
    pause_button.connect_clicked(move |_| {
        let mut state = state_clone.borrow_mut();
        if state.recording {
            state.paused = !state.paused;
        }
    });
    controls.append(&pause_button);

    // Timer label
    let timer_label = gtk4::Label::new(Some("00:00:00"));
    timer_label.add_css_class("timer-label");
    controls.append(&timer_label);

    // Record/Stop button
    let record_button = Button::with_label("⏺");
    let state_clone = Rc::clone(&state);
    let drawing_area_clone = drawing_area.clone();
    record_button.connect_clicked(move |button| {
        let mut state = state_clone.borrow_mut();
        if state.recording {
            state.stop_recording();
            button.set_label("⏺");
        } else {
            state.start_recording();
            button.set_label("⏹");
        }
        drawing_area_clone.queue_draw();
    });
    controls.append(&record_button);

    content.append(&controls);
    vbox.append(&content);
    window.set_child(Some(&vbox));
    
    // Hide window initially
    // Note: On first run, position the window manually in top-right corner
    // GNOME will remember this position for future sessions
    window.hide();
    
    // Note: We don't auto-hide on focus loss because it interferes with dragging
    // User can hide window by clicking tray icon again or using tray menu
    
    // Monitor visibility changes from tray icon and close button
    let window_clone = window.clone();
    let visible_clone = Arc::clone(&visible);
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if let Ok(should_be_visible) = visible_clone.try_lock() {
            let is_visible = window_clone.is_visible();
            if *should_be_visible && !is_visible {
                println!("Showing window");
                window_clone.present();
            } else if !*should_be_visible && is_visible {
                println!("Hiding window");
                window_clone.hide();
            }
        }
        glib::ControlFlow::Continue
    });

    // Update timer and waveform
    let state_clone = Rc::clone(&state);
    let timer_label_clone = timer_label.clone();
    let drawing_area_clone = drawing_area.clone();
    glib::timeout_add_local(Duration::from_millis(50), move || {
        let mut state = state_clone.borrow_mut();
        if state.recording && !state.paused {
            if let Some(start) = state.start_time {
                state.elapsed = start.elapsed();
                let secs = state.elapsed.as_secs();
                let mins = secs / 60;
                let secs = secs % 60;
                timer_label_clone.set_text(&format!("00:{:02}:{:02}", mins, secs));
            }
        }
        drawing_area_clone.queue_draw();
        glib::ControlFlow::Continue
    });
}

fn show_settings_dialog(parent: &ApplicationWindow, state: &Rc<RefCell<RecorderState>>) {
    use gtk4::{Dialog, Label, ComboBoxText, Box as GtkBox, ResponseType, Button};
    
    let dialog = Dialog::builder()
        .title("Settings")
        .transient_for(parent)
        .modal(true)
        .default_width(340)
        .default_height(200)
        .build();
    
    // Create custom button box with spacing
    let button_box = GtkBox::new(Orientation::Horizontal, 12);
    button_box.set_halign(gtk4::Align::End);
    button_box.set_margin_top(8);
    button_box.set_margin_bottom(8);
    button_box.set_margin_start(12);
    button_box.set_margin_end(12);
    
    let cancel_button = Button::with_label("Cancel");
    cancel_button.add_css_class("settings-button");
    let cancel_dialog = dialog.clone();
    cancel_button.connect_clicked(move |_| {
        cancel_dialog.response(ResponseType::Cancel);
    });
    
    let apply_button = Button::with_label("Apply");
    apply_button.add_css_class("settings-button");
    apply_button.add_css_class("suggested-action");
    let apply_dialog = dialog.clone();
    apply_button.connect_clicked(move |_| {
        apply_dialog.response(ResponseType::Apply);
    });
    
    button_box.append(&cancel_button);
    button_box.append(&apply_button);
    
    let content_area = dialog.content_area();
    let vbox = GtkBox::new(Orientation::Vertical, 8);
    vbox.set_margin_top(12);
    vbox.set_margin_bottom(12);
    vbox.set_margin_start(12);
    vbox.set_margin_end(12);
    
    // Microphone selection section
    let mic_label = Label::builder()
        .label("<small>Microphone</small>")
        .use_markup(true)
        .halign(gtk4::Align::Start)
        .build();
    mic_label.add_css_class("settings-label");
    vbox.append(&mic_label);
    
    let mic_combo = ComboBoxText::new();
    mic_combo.add_css_class("settings-combo");
    let state_borrow = state.borrow();
    let mut selected_mic_idx = 0;
    let mut combo_idx = 0;
    
    for (idx, source) in state_borrow.available_sources.iter().enumerate() {
        if !source.is_monitor {
            mic_combo.append(Some(&idx.to_string()), &source.display_name);
            if idx == state_borrow.selected_mic_index {
                selected_mic_idx = combo_idx;
            }
            combo_idx += 1;
        }
    }
    mic_combo.set_active(Some(selected_mic_idx as u32));
    vbox.append(&mic_combo);
    
    // System audio selection
    let loopback_label = Label::builder()
        .label("<small>System Audio (Loopback)</small>")
        .use_markup(true)
        .halign(gtk4::Align::Start)
        .margin_top(6)
        .build();
    loopback_label.add_css_class("settings-label");
    vbox.append(&loopback_label);
    
    let loopback_combo = ComboBoxText::new();
    loopback_combo.add_css_class("settings-combo");
    loopback_combo.append(Some("none"), "None");
    
    let mut selected_loopback_idx = 0;
    combo_idx = 1; // Start at 1 because 0 is "None"
    
    for (idx, source) in state_borrow.available_sources.iter().enumerate() {
        if source.is_monitor {
            loopback_combo.append(Some(&idx.to_string()), &source.display_name);
            if state_borrow.selected_loopback_index == Some(idx) {
                selected_loopback_idx = combo_idx;
            }
            combo_idx += 1;
        }
    }
    
    if state_borrow.selected_loopback_index.is_none() {
        loopback_combo.set_active(Some(0));
    } else {
        loopback_combo.set_active(Some(selected_loopback_idx as u32));
    }
    
    vbox.append(&loopback_combo);
    
    // Microphone gain
    let gain_label = Label::builder()
        .label("<small>Microphone Gain</small>")
        .use_markup(true)
        .halign(gtk4::Align::Start)
        .margin_top(6)
        .build();
    gain_label.add_css_class("settings-label");
    vbox.append(&gain_label);
    
    let gain_value = *state_borrow.mic_gain.lock().unwrap();
    let gain_db = if gain_value > 0.0 { 20.0 * (gain_value as f64).log10() } else { -60.0 };
    
    let gain_box = GtkBox::new(Orientation::Horizontal, 6);
    let gain_scale = gtk4::Scale::with_range(Orientation::Horizontal, -20.0, 20.0, 1.0);
    gain_scale.add_css_class("settings-scale");
    gain_scale.set_value(gain_db as f64);
    gain_scale.set_hexpand(true);
    
    let gain_value_label = Label::builder()
        .label(&format!("{:+.1} dB", gain_db))
        .width_chars(7)
        .build();
    gain_value_label.add_css_class("settings-label");
    let mic_gain_clone = Arc::clone(&state_borrow.mic_gain);
    let label_clone = gain_value_label.clone();
    
    gain_scale.connect_value_changed(move |scale| {
        let db = scale.value();
        let gain = 10_f64.powf(db / 20.0);
        *mic_gain_clone.lock().unwrap() = gain as f32;
        label_clone.set_text(&format!("{:+.1} dB", db));
    });
    
    gain_box.append(&gain_scale);
    gain_box.append(&gain_value_label);
    vbox.append(&gain_box);
    
    // Save directory section
    let save_dir_label = Label::builder()
        .label("<small>Dossier d'enregistrement</small>")
        .use_markup(true)
        .halign(gtk4::Align::Start)
        .margin_top(6)
        .build();
    save_dir_label.add_css_class("settings-label");
    vbox.append(&save_dir_label);
    
    let save_dir_box = GtkBox::new(Orientation::Horizontal, 6);
    let save_dir_entry = gtk4::Entry::new();
    save_dir_entry.set_hexpand(true);
    save_dir_entry.add_css_class("settings-entry");
    let current_dir = state_borrow.save_directory.lock().unwrap().clone()
        .unwrap_or_else(|| "Dossier courant".to_string());
    save_dir_entry.set_text(&current_dir);
    save_dir_entry.set_placeholder_text(Some("Dossier courant"));
    
    let browse_button = Button::with_label("...");
    browse_button.add_css_class("settings-button");
    browse_button.set_tooltip_text(Some("Parcourir"));
    
    let parent_for_picker = parent.clone();
    let entry_for_picker = save_dir_entry.clone();
    browse_button.connect_clicked(move |_| {
        use gtk4::{FileChooserNative, FileChooserAction, ResponseType};
        
        let file_chooser = FileChooserNative::new(
            Some("Sélectionner le dossier d'enregistrement"),
            Some(&parent_for_picker),
            FileChooserAction::SelectFolder,
            Some("Sélectionner"),
            Some("Annuler"),
        );
        
        let entry_clone = entry_for_picker.clone();
        file_chooser.connect_response(move |dialog, response| {
            if response == ResponseType::Accept {
                if let Some(file) = dialog.file() {
                    if let Some(path) = file.path() {
                        if let Some(path_str) = path.to_str() {
                            entry_clone.set_text(path_str);
                        }
                    }
                }
            }
        });
        
        file_chooser.show();
    });
    
    save_dir_box.append(&save_dir_entry);
    save_dir_box.append(&browse_button);
    vbox.append(&save_dir_box);
    
    // N8N Upload section
    let n8n_label = Label::builder()
        .label("<small>Upload N8N</small>")
        .use_markup(true)
        .halign(gtk4::Align::Start)
        .margin_top(10)
        .build();
    n8n_label.add_css_class("settings-label");
    vbox.append(&n8n_label);
    
    let n8n_enabled_box = GtkBox::new(Orientation::Horizontal, 6);
    let n8n_enabled_check = gtk4::CheckButton::new();
    n8n_enabled_check.set_active(*state_borrow.n8n_enabled.lock().unwrap());
    let n8n_enabled_label = Label::builder()
        .label("Activer l'upload vers N8N")
        .halign(gtk4::Align::Start)
        .build();
    n8n_enabled_label.add_css_class("settings-label");
    n8n_enabled_box.append(&n8n_enabled_check);
    n8n_enabled_box.append(&n8n_enabled_label);
    vbox.append(&n8n_enabled_box);
    
    let n8n_endpoint_label = Label::builder()
        .label("<small>URL de l'endpoint N8N</small>")
        .use_markup(true)
        .halign(gtk4::Align::Start)
        .margin_top(4)
        .build();
    n8n_endpoint_label.add_css_class("settings-label");
    vbox.append(&n8n_endpoint_label);
    
    let n8n_endpoint_entry = gtk4::Entry::new();
    n8n_endpoint_entry.add_css_class("settings-entry");
    n8n_endpoint_entry.set_placeholder_text(Some("https://..."));
    if let Some(endpoint) = state_borrow.n8n_endpoint.lock().unwrap().clone() {
        n8n_endpoint_entry.set_text(&endpoint);
    }
    n8n_endpoint_entry.set_sensitive(*state_borrow.n8n_enabled.lock().unwrap());
    
    let endpoint_entry_clone = n8n_endpoint_entry.clone();
    n8n_enabled_check.connect_toggled(move |check| {
        endpoint_entry_clone.set_sensitive(check.is_active());
    });
    
    vbox.append(&n8n_endpoint_entry);
    
    let n8n_save_locally_box = GtkBox::new(Orientation::Horizontal, 6);
    let n8n_save_locally_check = gtk4::CheckButton::new();
    n8n_save_locally_check.set_active(*state_borrow.save_locally.lock().unwrap());
    let n8n_save_locally_label = Label::builder()
        .label("Conserver le fichier localement après upload")
        .halign(gtk4::Align::Start)
        .build();
    n8n_save_locally_label.add_css_class("settings-label");
    n8n_save_locally_box.append(&n8n_save_locally_check);
    n8n_save_locally_box.append(&n8n_save_locally_label);
    n8n_save_locally_box.set_margin_top(4);
    vbox.append(&n8n_save_locally_box);
    
    drop(state_borrow); // Release borrow before showing dialog
    
    content_area.append(&vbox);
    content_area.append(&button_box);
    
    let state_clone = Rc::clone(state);
    dialog.connect_response(move |dialog, response| {
        if response == ResponseType::Apply || response == ResponseType::Cancel {
            if response == ResponseType::Apply {
                let mut state = state_clone.borrow_mut();
                
                // Update microphone
                if let Some(id) = mic_combo.active_id() {
                    if let Ok(idx) = id.parse::<usize>() {
                        state.selected_mic_index = idx;
                        println!("Microphone updated to index: {}", idx);
                    }
                }
                
                // Update loopback
                if let Some(id) = loopback_combo.active_id() {
                    if id == "none" {
                        state.selected_loopback_index = None;
                        println!("Loopback disabled");
                    } else if let Ok(idx) = id.parse::<usize>() {
                        state.selected_loopback_index = Some(idx);
                        println!("Loopback updated to index: {}", idx);
                    }
                }
                
                // Update save directory
                let dir_text = save_dir_entry.text().to_string();
                if dir_text.is_empty() || dir_text == "Dossier courant" {
                    *state.save_directory.lock().unwrap() = None;
                } else {
                    // Expand ~ to home directory
                    let expanded_path = if dir_text.starts_with("~/") {
                        if let Some(home) = dirs::home_dir() {
                            home.join(&dir_text[2..]).to_string_lossy().to_string()
                        } else {
                            dir_text
                        }
                    } else {
                        dir_text
                    };
                    *state.save_directory.lock().unwrap() = Some(expanded_path);
                    println!("Save directory updated");
                }
                
                // Update N8N settings
                *state.n8n_enabled.lock().unwrap() = n8n_enabled_check.is_active();
                
                let endpoint_text = n8n_endpoint_entry.text().to_string();
                if endpoint_text.is_empty() {
                    *state.n8n_endpoint.lock().unwrap() = None;
                } else {
                    *state.n8n_endpoint.lock().unwrap() = Some(endpoint_text);
                    println!("N8N endpoint updated");
                }
                
                *state.save_locally.lock().unwrap() = n8n_save_locally_check.is_active();
                
                // Save config
                state.save_config();
            }
            dialog.close();
        }
    });
    
    dialog.present();
}

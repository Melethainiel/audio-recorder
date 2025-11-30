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

const APP_ID: &str = "com.audio.recorder";

// Global window reference for tray icon to toggle
static WINDOW_VISIBLE: Mutex<Option<Arc<Mutex<bool>>>> = Mutex::new(None);

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
}

impl RecorderState {
    fn new() -> Self {
        let available_sources = Self::get_available_sources();
        
        let selected_mic_index = available_sources
            .iter()
            .position(|s| !s.is_monitor && (s.name == "pulse" || s.name == "pipewire"))
            .or_else(|| available_sources.iter().position(|s| !s.is_monitor))
            .unwrap_or(0);
        
        let selected_loopback_index = available_sources
            .iter()
            .position(|s| s.is_monitor);
        
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
            mic_gain: Arc::new(Mutex::new(1.0)),
        }
    }
    
    fn get_available_sources() -> Vec<AudioSource> {
        let mut sources = Vec::new();
        
        if let Ok(output) = std::process::Command::new("pactl")
            .args(["list", "sources", "short"])
            .output()
        {
            if output.status.success() {
                if let Ok(stdout) = String::from_utf8(output.stdout) {
                    for line in stdout.lines() {
                        let parts: Vec<&str> = line.split('\t').collect();
                        if parts.len() >= 2 {
                            let name = parts[1].to_string();
                            let is_monitor = name.contains(".monitor");
                            let display_name = name
                                .replace("alsa_input.", "")
                                .replace("alsa_output.", "")
                                .replace(".monitor", " (Monitor)")
                                .replace("_", " ");
                            
                            sources.push(AudioSource {
                                name,
                                display_name,
                                is_monitor,
                            });
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
            
            if let Ok(mut file) = File::create(&filename) {
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
                            println!("Saved: {}", filename);
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

    // Main container
    let vbox = gtk4::Box::new(Orientation::Vertical, 10);
    vbox.set_margin_top(16);
    vbox.set_margin_bottom(16);
    vbox.set_margin_start(16);
    vbox.set_margin_end(16);
    
    // Add the drag gesture to vbox so user can drag from anywhere
    vbox.add_controller(gesture);

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
    
    vbox.append(&drawing_area);

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

    vbox.append(&controls);
    window.set_child(Some(&vbox));

    // Setup window visibility control from tray icon
    let visible = Arc::new(Mutex::new(false)); // Start hidden
    *WINDOW_VISIBLE.lock().unwrap() = Some(Arc::clone(&visible));
    
    // Hide window initially
    // Note: On first run, position the window manually in top-right corner
    // GNOME will remember this position for future sessions
    window.hide();
    
    // Note: We don't auto-hide on focus loss because it interferes with dragging
    // User can hide window by clicking tray icon again or using tray menu
    
    // Monitor visibility changes from tray icon
    let window_clone = window.clone();
    let visible_clone = Arc::clone(&visible);
    glib::timeout_add_local(Duration::from_millis(100), move || {
        if let Ok(should_be_visible) = visible_clone.lock() {
            let is_visible = window_clone.is_visible();
            if *should_be_visible && !is_visible {
                window_clone.present();
            } else if !*should_be_visible && is_visible {
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

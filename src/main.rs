use eframe::egui;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use chrono::Local;
use vorbis_rs::{VorbisEncoderBuilder};
use std::fs::File;

fn main() -> Result<(), eframe::Error> {
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
    
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([380.0, 130.0])
            .with_resizable(false)
            .with_decorations(true)
            .with_title_shown(false),
        ..Default::default()
    };
    
    eframe::run_native(
        "Audio Recorder",
        options,
        Box::new(move |_| {
            Ok(Box::new(RecorderApp::default()))
        }),
    )
}

struct AudioSource {
    name: String,
    display_name: String,
    is_monitor: bool,
}

struct RecorderApp {
    recording: bool,
    paused: bool,
    show_settings: bool,
    input_stream: Option<cpal::Stream>,
    output_stream: Option<cpal::Stream>,
    start_time: Option<Instant>,
    elapsed: Duration,
    input_level: Arc<Mutex<f32>>,
    output_level: Arc<Mutex<f32>>,
    input_samples: Arc<Mutex<Vec<f32>>>,
    output_samples: Arc<Mutex<Vec<f32>>>,
    waveform_history: Arc<Mutex<Vec<f32>>>,
    sample_rate: u32,
    channels: u16,
    // Audio source selection
    available_sources: Vec<AudioSource>,
    selected_mic_index: usize,
    selected_loopback_index: Option<usize>,
    // Gain control
    mic_gain: Arc<Mutex<f32>>,
}

impl RecorderApp {
    fn new() -> Self {
        let available_sources = Self::get_available_sources();
        
        // Find default mic (first non-monitor, preferably "pulse" or "pipewire")
        let selected_mic_index = available_sources
            .iter()
            .position(|s| !s.is_monitor && (s.name == "pulse" || s.name == "pipewire"))
            .or_else(|| available_sources.iter().position(|s| !s.is_monitor))
            .unwrap_or(0);
        
        // Find default loopback (first monitor)
        let selected_loopback_index = available_sources
            .iter()
            .position(|s| s.is_monitor);
        
        Self {
            recording: false,
            paused: false,
            show_settings: false,
            input_stream: None,
            output_stream: None,
            start_time: None,
            elapsed: Duration::default(),
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
            mic_gain: Arc::new(Mutex::new(1.0)), // Default gain: 1.0 (no change)
        }
    }
    
    fn get_available_sources() -> Vec<AudioSource> {
        let mut sources = Vec::new();
        
        // Get PulseAudio/PipeWire sources via pactl
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
}

impl Default for RecorderApp {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for RecorderApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        
        // Update elapsed time
        if self.recording && !self.paused {
            if let Some(start) = self.start_time {
                self.elapsed = start.elapsed();
            }
        }
        
        // Settings window (separate viewport)
        if self.show_settings {
            ctx.show_viewport_immediate(
                egui::ViewportId::from_hash_of("settings"),
                egui::ViewportBuilder::default()
                    .with_title("Settings")
                    .with_inner_size([320.0, 300.0])
                    .with_resizable(false),
                |ctx, _class| {
                    // Apply same visual style
                    let mut style = (*ctx.style()).clone();
                    style.visuals.window_fill = egui::Color32::WHITE;
                    style.visuals.panel_fill = egui::Color32::WHITE;
                    style.visuals.widgets.noninteractive.bg_fill = egui::Color32::from_gray(245);
                    style.visuals.widgets.inactive.bg_fill = egui::Color32::from_gray(240);
                    style.visuals.widgets.hovered.bg_fill = egui::Color32::from_gray(230);
                    style.visuals.widgets.active.bg_fill = egui::Color32::from_rgb(59, 130, 246);
                    style.visuals.selection.bg_fill = egui::Color32::from_rgb(59, 130, 246);
                    ctx.set_style(style);
                    
                    let panel_frame = egui::Frame::default()
                        .fill(egui::Color32::WHITE)
                        .inner_margin(20.0);
                    
                    egui::CentralPanel::default().frame(panel_frame).show(ctx, |ui| {
                        // Title
                        ui.horizontal(|ui| {
                            ui.label(egui::RichText::new("Settings").size(18.0).strong()
                                .color(egui::Color32::from_gray(60)));
                        });
                        
                        ui.add_space(16.0);
                        
                        // Microphone selection
                        ui.label(egui::RichText::new("Microphone")
                            .color(egui::Color32::from_gray(80)));
                        ui.add_space(4.0);
                        let mic_sources: Vec<_> = self.available_sources
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| !s.is_monitor)
                            .collect();
                        
                        let current_mic = mic_sources
                            .iter()
                            .find(|(i, _)| *i == self.selected_mic_index)
                            .map(|(_, s)| s.display_name.as_str())
                            .unwrap_or("None");
                        
                        egui::ComboBox::from_id_salt("mic_select")
                            .selected_text(current_mic)
                            .width(260.0)
                            .show_ui(ui, |ui| {
                                for (idx, source) in &mic_sources {
                                    ui.selectable_value(
                                        &mut self.selected_mic_index,
                                        *idx,
                                        &source.display_name
                                    );
                                }
                            });
                        
                        ui.add_space(14.0);
                        
                        // System audio selection
                        ui.label(egui::RichText::new("System Audio")
                            .color(egui::Color32::from_gray(80)));
                        ui.add_space(4.0);
                        let loopback_sources: Vec<_> = self.available_sources
                            .iter()
                            .enumerate()
                            .filter(|(_, s)| s.is_monitor)
                            .collect();
                        
                        let current_loopback = self.selected_loopback_index
                            .and_then(|i| self.available_sources.get(i))
                            .map(|s| s.display_name.as_str())
                            .unwrap_or("None");
                        
                        egui::ComboBox::from_id_salt("loopback_select")
                            .selected_text(current_loopback)
                            .width(260.0)
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(self.selected_loopback_index.is_none(), "None").clicked() {
                                    self.selected_loopback_index = None;
                                }
                                for (idx, source) in &loopback_sources {
                                    let selected = self.selected_loopback_index == Some(*idx);
                                    if ui.selectable_label(selected, &source.display_name).clicked() {
                                        self.selected_loopback_index = Some(*idx);
                                    }
                                }
                            });
                        
                        ui.add_space(14.0);
                        
                        // Microphone gain slider
                        ui.label(egui::RichText::new("Microphone Gain")
                            .color(egui::Color32::from_gray(80)));
                        ui.add_space(4.0);
                        let mut gain = *self.mic_gain.lock().unwrap();
                        let gain_db = if gain > 0.0 { 20.0 * gain.log10() } else { -60.0 };
                        ui.horizontal(|ui| {
                            ui.spacing_mut().slider_width = 180.0;
                            ui.add(egui::Slider::new(&mut gain, 0.1..=5.0)
                                .logarithmic(true)
                                .show_value(false));
                            ui.label(egui::RichText::new(format!("{:+.1} dB", gain_db))
                                .color(egui::Color32::from_gray(100)));
                        });
                        *self.mic_gain.lock().unwrap() = gain;
                        
                        ui.add_space(20.0);
                        
                        // Buttons
                        ui.horizontal(|ui| {
                            let button_style = |text: &str| {
                                egui::Button::new(
                                    egui::RichText::new(text).color(egui::Color32::from_gray(60))
                                )
                                .fill(egui::Color32::from_gray(240))
                                .rounding(6.0)
                                .min_size(egui::vec2(80.0, 32.0))
                            };
                            
                            if ui.add(button_style("Refresh")).clicked() {
                                self.available_sources = Self::get_available_sources();
                            }
                            
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                if ui.add(
                                    egui::Button::new(
                                        egui::RichText::new("Close").color(egui::Color32::WHITE)
                                    )
                                    .fill(egui::Color32::from_rgb(59, 130, 246))
                                    .rounding(6.0)
                                    .min_size(egui::vec2(80.0, 32.0))
                                ).clicked() {
                                    self.show_settings = false;
                                }
                            });
                        });
                    });
                    
                    if ctx.input(|i| i.viewport().close_requested()) {
                        self.show_settings = false;
                    }
                },
            );
        }
        
        let panel_frame = egui::Frame::default()
            .fill(egui::Color32::WHITE)
            .inner_margin(16.0);
        
        egui::CentralPanel::default().frame(panel_frame).show(ctx, |ui| {
            // Top bar with settings button
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(
                        egui::Button::new(egui::RichText::new("⚙").size(16.0))
                            .fill(egui::Color32::TRANSPARENT)
                    ).clicked() && !self.recording {
                        self.show_settings = !self.show_settings;
                    }
                });
            });
            
            // Waveform visualization
            self.draw_waveform(ui);
            
            ui.add_space(12.0);
            
            // Controls row: [Pause] [Timer] [Stop]
            ui.horizontal(|ui| {
                let available_width = ui.available_width();
                let button_size = 44.0;
                let timer_width = 80.0;
                let total_content = button_size * 2.0 + timer_width + 40.0;
                let side_padding = (available_width - total_content) / 2.0;
                
                ui.add_space(side_padding);
                
                // Pause button (blue circle with pause icon)
                let pause_color = egui::Color32::from_rgb(59, 130, 246); // Blue
                let (pause_rect, pause_response) = ui.allocate_exact_size(
                    egui::vec2(button_size, button_size),
                    egui::Sense::click()
                );
                
                if ui.is_rect_visible(pause_rect) {
                    let painter = ui.painter();
                    painter.circle_filled(pause_rect.center(), button_size / 2.0, pause_color);
                    
                    if self.paused {
                        // Play triangle
                        let size = 12.0;
                        let center = pause_rect.center();
                        let points = vec![
                            egui::pos2(center.x - size * 0.4, center.y - size * 0.6),
                            egui::pos2(center.x - size * 0.4, center.y + size * 0.6),
                            egui::pos2(center.x + size * 0.6, center.y),
                        ];
                        painter.add(egui::Shape::convex_polygon(points, egui::Color32::WHITE, egui::Stroke::NONE));
                    } else {
                        // Pause bars
                        let bar_width = 4.0;
                        let bar_height = 14.0;
                        let gap = 4.0;
                        let center = pause_rect.center();
                        
                        painter.rect_filled(
                            egui::Rect::from_center_size(
                                egui::pos2(center.x - gap / 2.0 - bar_width / 2.0, center.y),
                                egui::vec2(bar_width, bar_height)
                            ),
                            2.0,
                            egui::Color32::WHITE
                        );
                        painter.rect_filled(
                            egui::Rect::from_center_size(
                                egui::pos2(center.x + gap / 2.0 + bar_width / 2.0, center.y),
                                egui::vec2(bar_width, bar_height)
                            ),
                            2.0,
                            egui::Color32::WHITE
                        );
                    }
                }
                
                if pause_response.clicked() && self.recording {
                    self.paused = !self.paused;
                }
                
                ui.add_space(20.0);
                
                // Timer
                let secs = self.elapsed.as_secs();
                let mins = secs / 60;
                let secs = secs % 60;
                let timer_text = format!("{:02}:{:02}:{:02}", 0, mins, secs);
                
                ui.allocate_ui_with_layout(
                    egui::vec2(timer_width, button_size),
                    egui::Layout::centered_and_justified(egui::Direction::LeftToRight),
                    |ui| {
                        ui.label(
                            egui::RichText::new(timer_text)
                                .size(20.0)
                                .color(egui::Color32::from_rgb(100, 100, 100))
                                .strong()
                        );
                    }
                );
                
                ui.add_space(20.0);
                
                // Stop/Record button (red rounded square)
                let stop_color = egui::Color32::from_rgb(239, 68, 68); // Red
                let (stop_rect, stop_response) = ui.allocate_exact_size(
                    egui::vec2(button_size, button_size),
                    egui::Sense::click()
                );
                
                if ui.is_rect_visible(stop_rect) {
                    let painter = ui.painter();
                    painter.rect_filled(stop_rect, 12.0, stop_color);
                    
                    if self.recording {
                        // Stop square
                        let square_size = 14.0;
                        painter.rect_filled(
                            egui::Rect::from_center_size(stop_rect.center(), egui::vec2(square_size, square_size)),
                            3.0,
                            egui::Color32::WHITE
                        );
                    } else {
                        // Record circle
                        painter.circle_filled(stop_rect.center(), 8.0, egui::Color32::WHITE);
                    }
                }
                
                if stop_response.clicked() {
                    if self.recording {
                        self.stop_recording();
                    } else {
                        self.start_recording();
                    }
                }
            });
        });
        
        ctx.request_repaint_after(Duration::from_millis(50));
    }
}

impl RecorderApp {
    fn draw_waveform(&self, ui: &mut egui::Ui) {
        let available_width = ui.available_width();
        let height = 50.0;
        let num_bars = 60;
        let bar_width = 3.0;
        let spacing = (available_width - (num_bars as f32 * bar_width)) / (num_bars as f32 + 1.0);
        
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(available_width, height),
            egui::Sense::hover()
        );
        
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();
            
            // Background
            painter.rect_filled(
                rect,
                8.0,
                egui::Color32::from_rgb(243, 244, 246) // Light gray
            );
            
            // Get recent audio levels for waveform
            let waveform_data = self.waveform_history.lock().unwrap();
            let current_level = *self.input_level.lock().unwrap();
            
            let green = egui::Color32::from_rgb(34, 197, 94);
            let gray = egui::Color32::from_rgb(209, 213, 219);
            
            for i in 0..num_bars {
                let x = rect.left() + spacing + (i as f32 * (bar_width + spacing));
                
                // Get level from history or use current
                let level = if i < waveform_data.len() {
                    waveform_data[i]
                } else if self.recording && !self.paused {
                    current_level * (0.5 + 0.5 * ((i as f32 * 0.3).sin().abs()))
                } else {
                    0.0
                };
                
                let bar_height = (level * height * 0.8).max(4.0).min(height * 0.9);
                let bar_rect = egui::Rect::from_center_size(
                    egui::pos2(x + bar_width / 2.0, rect.center().y),
                    egui::vec2(bar_width, bar_height)
                );
                
                let color = if self.recording && level > 0.05 { green } else { gray };
                painter.rect_filled(bar_rect, 1.5, color);
            }
        }
    }
    
    fn start_recording(&mut self) {
        let host = cpal::default_host();
        
        self.start_time = Some(Instant::now());
        self.elapsed = Duration::default();
        
        // Get selected microphone source name
        let mic_source_name = self.available_sources
            .get(self.selected_mic_index)
            .map(|s| s.name.clone());
        
        if let Some(mic_name) = mic_source_name {
            println!("Using microphone: {}", mic_name);
            
            // Set PULSE_SOURCE for mic
            // SAFETY: Single-threaded at this point
            unsafe { std::env::set_var("PULSE_SOURCE", &mic_name) };
            
            // Find pulse device
            let pulse_device = host.input_devices()
                .ok()
                .and_then(|mut devices| {
                    devices.find(|d| d.name().map(|n| n == "pulse").unwrap_or(false))
                });
            
            if let Some(input_device) = pulse_device {
                if let Ok(input_config) = input_device.default_input_config() {
                    self.sample_rate = input_config.sample_rate().0;
                    self.channels = input_config.channels();
                    println!("Mic: {} Hz, {} channel(s)", self.sample_rate, self.channels);
                    
                    let input_level = Arc::clone(&self.input_level);
                    let input_samples = Arc::clone(&self.input_samples);
                    let waveform_history = Arc::clone(&self.waveform_history);
                    let output_level = Arc::clone(&self.output_level);
                    let mic_gain = Arc::clone(&self.mic_gain);
                    
                    if let Ok(mic_stream) = input_device.build_input_stream(
                        &input_config.into(),
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            let gain = *mic_gain.lock().unwrap();
                            
                            // Apply gain to samples
                            let gained_data: Vec<f32> = data.iter()
                                .map(|&s| (s * gain).clamp(-1.0, 1.0))
                                .collect();
                            
                            let sum: f32 = gained_data.iter().map(|&s| s * s).sum();
                            let rms = (sum / gained_data.len() as f32).sqrt();
                            let mic_level = (rms * 5.0).min(1.0);
                            *input_level.lock().unwrap() = mic_level;
                            
                            // Combine mic and loopback levels for waveform
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
            
            // SAFETY: Cleaning up
            unsafe { std::env::remove_var("PULSE_SOURCE") };
        }
        
        // System audio loopback
        let loopback_source_name = self.selected_loopback_index
            .and_then(|i| self.available_sources.get(i))
            .map(|s| s.name.clone());
        
        if let Some(loopback_name) = loopback_source_name {
            println!("Using loopback: {}", loopback_name);
            
            // SAFETY: Single-threaded at this point
            unsafe { std::env::set_var("PULSE_SOURCE", &loopback_name) };
            
            let pulse_device = host.input_devices()
                .ok()
                .and_then(|mut devices| {
                    devices.find(|d| d.name().map(|n| n == "pulse").unwrap_or(false))
                });
            
            if let Some(loopback_device) = pulse_device {
                if let Ok(loopback_config) = loopback_device.default_input_config() {
                    println!("Loopback: {} Hz, {} channel(s)", 
                        loopback_config.sample_rate().0, loopback_config.channels());
                    
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
                            
                            // Convert to mono
                            let mono: Vec<f32> = data
                                .chunks(source_channels as usize)
                                .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
                                .collect();
                            
                            // Simple resampling if sample rates differ
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
                        println!("Loopback capture enabled");
                    }
                }
            }
            
            // SAFETY: Cleaning up
            unsafe { std::env::remove_var("PULSE_SOURCE") };
        }
        
        self.recording = true;
        println!("Recording started");
    }

fn stop_recording(&mut self) {
    // Stop both streams
    if let Some(stream) = self.input_stream.take() {
        drop(stream);
    }
    if let Some(stream) = self.output_stream.take() {
        drop(stream);
    }
    
    // Get samples from both sources
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
    
    // Convert mic to mono if stereo
    let mic_mono: Vec<f32> = if self.channels == 1 {
        mic_samples
    } else {
        mic_samples
            .chunks(self.channels as usize)
            .map(|chunk| chunk.iter().sum::<f32>() / chunk.len() as f32)
            .collect()
    };
    
    // Mix both sources together
    let max_len = mic_mono.len().max(system_samples.len());
    let mixed_samples: Vec<f32> = (0..max_len)
        .map(|i| {
            let mic = mic_mono.get(i).copied().unwrap_or(0.0);
            let sys = system_samples.get(i).copied().unwrap_or(0.0);
            // Mix with equal weight, clamp to prevent clipping
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
    
    // Reset waveform
    let mut history = self.waveform_history.lock().unwrap();
    history.iter_mut().for_each(|v| *v = 0.0);
}
    
}

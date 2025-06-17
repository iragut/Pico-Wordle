use eframe::egui;
use eframe::egui::{Button, CentralPanel, Visuals};
use std::io::Write;
use std::net::TcpStream;
use tokio::sync::mpsc::{self, Sender, Receiver};

struct KeyboardApp {
    tx: Sender<char>,
    status: String,
    status_rx: Receiver<String>,
}

impl KeyboardApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        _cc.egui_ctx.set_visuals(Visuals::light());

        // Create 2 chanels: one for sending key presses and one for status updates
        let (tx, mut rx) = mpsc::channel::<char>(100);
        let (status_tx, status_rx) = mpsc::channel::<String>(10);

        tokio::spawn(async move {
            loop {
                // Try to connect to the server
                match TcpStream::connect("169.254.1.1:6000") {
                    Ok(mut stream) => {
                        // Send the message too gui that is connected, and wait for received the keys
                        let _ = status_tx.send("Connected".to_string()).await;
                        while let Some(key) = rx.recv().await {
                            if let Err(e) = stream.write_all(&[key as u8]) {
                                eprintln!("Failed to send: {}", e);
                                let _ = status_tx.send(format!("Error: {}", e)).await;
                                break;
                            }
                            if let Err(e) = stream.flush() {
                                eprintln!("Failed to flush: {}", e);
                                let _ = status_tx.send(format!("Error: {}", e)).await;
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        eprintln!("Connection failed: {}", e);
                        let _ = status_tx.send(format!("Connection failed: {}", e)).await;
                        tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });

        Self {
            tx,
            status: "Connecting...".to_string(),
            status_rx,
        }
    }
}

impl eframe::App for KeyboardApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        while let Ok(status) = self.status_rx.try_recv() {
            self.status = status;
        }

        // Create the main UI layout for the keyboard
        CentralPanel::default().show(ctx, |ui| {
            ui.label(&self.status);

            
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    for c in "qwertyuiop".chars() {
                        if ui.add(Button::new(c.to_string()).min_size(egui::vec2(40.0, 40.0))).clicked() {
                            let _ = self.tx.try_send(c);
                        }
                        ui.add_space(4.0);
                    }
                });
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    for c in "asdfghjkl".chars() {
                        if ui.add(Button::new(c.to_string()).min_size(egui::vec2(40.0, 40.0))).clicked() {
                            let _ = self.tx.try_send(c);
                        }
                        ui.add_space(4.0);
                    }
                });
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    for c in "zxcvbnm".chars() {
                        if ui.add(Button::new(c.to_string()).min_size(egui::vec2(40.0, 40.0))).clicked() {
                            let _ = self.tx.try_send(c);
                        }
                        ui.add_space(4.0);
                    }
                });
                ui.add_space(4.0);

                ui.horizontal(|ui| {
                    if ui.add(Button::new("Backspace").min_size(egui::vec2(80.0, 40.0))).clicked() {
                        let _ = self.tx.try_send('D');
                    }
                    ui.add_space(4.0);
                    if ui.add(Button::new("Enter").min_size(egui::vec2(80.0, 40.0))).clicked() {
                        let _ = self.tx.try_send('E');
                    }
                });
            });
        });

        ctx.request_repaint();
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let options = eframe::NativeOptions {
        initial_window_size: Some(egui::vec2(600.0, 300.0)),
        ..Default::default()
    };
    eframe::run_native(
        "Keyboard",
        options,
        Box::new(|cc| Box::new(KeyboardApp::new(cc))),
    );
    Ok(())
}

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui;
use serde::Deserialize;
use std::sync::mpsc::{channel, Receiver};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tungstenite::{connect, Message};
use url::Url;

#[derive(Deserialize, Debug, Clone)]
struct GlazeEnvelope {
  #[serde(rename = "messageType")]
  message_type: String,
  data: serde_json::Value, 
}

#[derive(Deserialize, Debug, Clone)]
struct Workspace {
  name: String,

  #[serde(default, alias = "hasFocus")]
  focused: bool,

  #[serde(default, alias = "isDisplayed")]
  visible: bool,
}

#[derive(Deserialize, Debug, Clone)]
struct WorkspacesData {
  workspaces: Vec<Workspace>,
}

fn main() -> eframe::Result<()> {
  let native_options = eframe::NativeOptions {
    viewport: egui::ViewportBuilder::default()
      .with_decorations(false)
      .with_always_on_top()
      .with_taskbar(false)
      .with_inner_size([1920.0, 32.0])
      .with_position([0.0, 0.0]),
      ..Default::default()
  };

  eframe::run_native(
    "QueBar",
    native_options,
    Box::new(|cc| {
      let ctx = cc.egui_ctx.clone();
      let ctx_bat = cc.egui_ctx.clone();

      let (ws_tx, ws_rx) = channel();
      let (bat_tx, bat_rx) = channel();
      let repaint_signal = Arc::new(AtomicBool::new(false));
      let repaint_signal_ws = repaint_signal.clone();

      std::thread::spawn(move || {
        let url = Url::parse("ws://localhost:6123").unwrap();
        loop {
          match connect(url.as_str()) {
            Ok((mut socket, _)) => {
              let _ = socket.send(Message::Text("sub -e workspace_activated".into()));
              let _ = socket.send(Message::Text("sub -e focus_changed".into()));
              let _ = socket.send(Message::Text("query workspaces".into()));

              loop {
                match socket.read() {
                  Ok(msg) => {
                    if let Message::Text(text) = msg {
                      if let Ok(envelope) = serde_json::from_str::<GlazeEnvelope>(&text) {
                        match envelope.message_type.as_str() {
                          "client_response" | "query_response" => {
                            if envelope.data.get("subscriptionId").is_some() { continue; }

                            if let Ok(d) = serde_json::from_value::<WorkspacesData>(envelope.data) {
                              let _ = ws_tx.send(d.workspaces);
                              repaint_signal_ws.store(true, Ordering::Relaxed); 
                            }
                          }
                          "event" | "subscribed_event" | "event_subscription" => {
                            let _ = socket.send(Message::Text("query workspaces".into()));
                          }
                          _ => {}
                        }
                      }
                    }
                  }
                  Err(_) => break,
                }
              }
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_secs(2)),
          }
        }
      });

      std::thread::spawn(move || {
        loop {
          if repaint_signal.swap(false, Ordering::Relaxed) {
            ctx.request_repaint();
          }
          std::thread::sleep(std::time::Duration::from_millis(100));
        }
      });
    
      std::thread::spawn(move || {
        let manager = battery::Manager::new().ok();
        loop {
          if let Some(ref mgr) = manager {
            if let Ok(mut bats) = mgr.batteries() {
              if let Some(Ok(bat)) = bats.next() {
                let pct = bat.state_of_charge().get::<battery::units::ratio::percent>();
                let _ = bat_tx.send(format!("{:.0}%", pct));
                ctx_bat.request_repaint(); // <--- WAKE UP UI!
              }
            }
          }
          std::thread::sleep(std::time::Duration::from_secs(60));
        }
      });

      Ok(Box::new(MyTaskbar::new(ws_rx, bat_rx)))
    }),
    )
}

struct MyTaskbar {
  date: String,
  time: String,
  battery_level: String,
  workspaces: Vec<Workspace>,
  ws_rx: Receiver<Vec<Workspace>>,
  bat_rx: Receiver<String>,
}

impl MyTaskbar {
  fn new(ws_rx: Receiver<Vec<Workspace>>, bat_rx: Receiver<String>) -> Self {
    Self {
      date: String::new(),
      time: String::new(),
      battery_level: "100%".into(),
      workspaces: Vec::new(),
      ws_rx,
      bat_rx,
    }
  }
}

impl eframe::App for MyTaskbar {
  fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
    let mut repaint_needed = false;

    while let Ok(ws) = self.ws_rx.try_recv() {
      self.workspaces = ws;
      repaint_needed = true;
    }

    while let Ok(bat) = self.bat_rx.try_recv() {
      self.battery_level = bat;
      repaint_needed = true;
    }

    let now = chrono::Local::now();
    let new_time = now.format("%H:%M").to_string();
    let new_date = now.format("%m/%d/%Y").to_string();

    if new_time != self.time || new_date != self.date {
      self.time = new_time;
      self.date = new_date;
      repaint_needed = true;
    }

    let panel_frame = egui::Frame::NONE
      .fill(egui::Color32::from_black_alpha(180))
      .inner_margin(5.0);

    egui::TopBottomPanel::top("taskbar_panel")
      .frame(panel_frame)
      .show(ctx, |ui| {
        ui.horizontal(|ui| {
          ui.visuals_mut().override_text_color = Some(egui::Color32::WHITE);

          ui.separator();
          ui.label("🐓 QueBar");
          ui.separator();
          // In your update loop:
          for ws in &self.workspaces {
            // Update to use the new fields .focused and .visible
            let (text_color, bg_color) = match (ws.focused, ws.visible) {
              (true, _) => (egui::Color32::WHITE, egui::Color32::from_rgb(70, 70, 180)),
              (false, true) => (egui::Color32::LIGHT_GRAY, egui::Color32::from_black_alpha(80)),
              _ => (egui::Color32::GRAY, egui::Color32::TRANSPARENT),
            };

            let _resp = egui::Frame::NONE
              .fill(bg_color)
              .corner_radius(4) // FIXED: Replaced .rounding(4.0)
              .inner_margin(egui::Margin::symmetric(10, 2))
              .show(ui, |ui| ui.colored_label(text_color, &ws.name))
              .response;
          }
          ui.separator();
          ui.with_layout(egui::Layout::right_to_left(egui::Align::Max), |ui| {
            ui.separator();
            ui.label(format!("🔋 {}  ", &self.battery_level));
            ui.separator();
            ui.label(&self.time);
            ui.separator();
            ui.label(&self.date);
            ui.separator();
          });
        });
      });

    if repaint_needed {
      ctx.request_repaint(); 
    }
    let seconds_until_next_minute = 60 - ((now.timestamp() as u64) % 60);

    ctx.request_repaint_after(std::time::Duration::from_secs(seconds_until_next_minute));
  }
}

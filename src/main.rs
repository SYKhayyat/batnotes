use eframe::egui;

fn main() {
    let options = eframe::NativeOptions::default();
    eframe::run_native("batnotes", options, Box::new(|_cc| Box::new(App::default())));
}

#[derive(Default)]
struct App {
    text: String,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label("batnotes");
            ui.text_edit_multiline(&mut self.text);
        });
    }
}

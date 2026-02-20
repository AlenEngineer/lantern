use iced::{Element, Result, Task, run, widget::{button, column}, window::{self, Action, Id}};

#[derive(Debug, Default)]
struct State {  }

#[derive(Debug, Clone)]
enum Message {
    Exit,
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Exit => window::close(window::Id::unique()),
    }
}

fn view(state: &State) -> Element<'_, Message> {
    column![
        button("Exit").on_press(Message::Exit),
    ].into()
}

fn main() -> Result {
    run(update, view)
}

use iced::{
    Element, Task, application,
    widget::{button, column},
    window::{self},
};

#[derive(Debug, Default)]
struct State {
}

#[derive(Debug, Clone)]
enum Message {
    Exit,
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::Exit => {
            window::oldest().and_then(window::close)
        }
    }
}

fn view(state: &State) -> Element<'_, Message> {
    column![button("Exit").on_press(Message::Exit),].into()
}

fn main() -> iced::Result {
    application(State::default, update, view).run()
}

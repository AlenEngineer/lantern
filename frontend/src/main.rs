use iced::{
    Element, Task, application,
    widget::{button, column, row, text_input},
    window::{self},
};

#[derive(Debug, Default)]
struct State {
    search_query: String,
    available_videos: String,
    available_books: String,
}

#[derive(Debug, Clone)]
enum Message {
    SearchChanged(String),
    PerformSearch,
    Exit,
}

fn update(state: &mut State, message: Message) -> Task<Message> {
    match message {
        Message::SearchChanged(query) => {
            state.search_query = query;
            Task::none()
        }

        Message::PerformSearch => {
            println!("Searching {}", state.search_query);
            Task::none()
        }

        Message::Exit => window::oldest().and_then(window::close),
    }
}

fn view(state: &State) -> Element<'_, Message> {
    column![button("Exit").on_press(Message::Exit),
        row![text_input("Search a hymn or bible verse", &state.search_query)
            .on_input(Message::SearchChanged)
            .on_submit(Message::PerformSearch)]
    ].into()
}

fn main() -> iced::Result {
    application(State::default, update, view).run()
}

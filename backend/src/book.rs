enum BookContent {
    Json,
    Xml,
    Pdf,
}
pub struct Book {
    name: String,
    chapters: u32,
    content: BookContent,
    file_type: String,
    path: String,
}


pub mod mesh;
pub mod model;
pub mod print;
pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

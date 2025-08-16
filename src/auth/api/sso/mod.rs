pub mod client;
pub mod handler;

// Re-export the handler function for convenience
pub use handler::handle_callback;

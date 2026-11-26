//! The embedded single-page app.
//!
//! The HTML, CSS, JS and favicon are compiled into the binary with
//! `include_str!`, so `--web` needs no files on disk and no bundler. Editing
//! them is ordinary source editing; a rebuild picks the change up.

/// The page shell. Styles and script are separate files so the browser can cache
/// them independently of the markup.
pub const INDEX_HTML: &str = include_str!("page.html");
pub const APP_CSS: &str = include_str!("page.css");
pub const APP_JS: &str = include_str!("page.js");
pub const FAVICON_SVG: &str = include_str!("favicon.svg");

#![deny(unsafe_op_in_unsafe_fn)]
mod backend;
mod element;
mod render;
mod vnode;
mod wrappers;

pub use element::Element;
pub use render::{mount, render};
pub use wrappers::*;
pub mod manual_apis {
    pub use super::backend::WebBackend;
    pub use super::render::{put_node, render_in_node};
}

//! The line-level Markdown that the rules classify comment and doc lines by.

mod block_start;
mod line;

pub use block_start::BlockStart;
pub use block_start::HtmlBlockCondition;
pub use block_start::Preceding;
pub use line::Fence;
pub use line::FenceTracker;
pub use line::atx_header;
pub use line::bullet_marker;
pub use line::indent;
pub use line::link_definition;

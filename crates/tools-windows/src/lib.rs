//! Windows-specific tool implementations for the LingXi agent.
//!
//! Each tool wraps an existing capability from `assistant-windows` (UIA,
//! clipboard, QQ integration) or a standard library function (file IO,
//! shell execution) behind the platform-agnostic `Tool` trait.

pub mod clip_tools;
pub mod file_tools;
pub mod qq_tools;
pub mod shell_tools;
pub mod uia_tools;

use lingxi_tools::ToolRegistry;
use std::sync::Arc;

/// Register all default Windows tools into a registry.
pub fn register_default_tools(registry: &mut ToolRegistry) {
    // UIA / selection tools
    registry.register(Arc::new(uia_tools::ReadSelectionTool));
    registry.register(Arc::new(uia_tools::WriteTextTool));

    // QQ integration
    registry.register(Arc::new(qq_tools::QqReadSelectionTool));
    registry.register(Arc::new(qq_tools::QqWriteDraftTool));

    // Clipboard
    registry.register(Arc::new(clip_tools::ReadClipboardTool));
    registry.register(Arc::new(clip_tools::WriteClipboardTool));

    // File operations
    registry.register(Arc::new(file_tools::ReadFileTool));
    registry.register(Arc::new(file_tools::WriteFileTool));
    registry.register(Arc::new(file_tools::ListDirTool));

    // Shell
    registry.register(Arc::new(shell_tools::RunCommandTool));
}

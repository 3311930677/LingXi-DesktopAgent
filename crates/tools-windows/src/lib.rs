//! Windows-specific tool implementations for the LingXi agent.
//!
//! Each tool wraps an existing capability from `assistant-windows` (UIA,
//! clipboard, QQ integration) or a standard library function (file IO,
//! shell execution) behind the platform-agnostic `Tool` trait.

pub mod app_tools;
pub mod clip_tools;
pub mod file_tools;
pub mod input_tools;
pub mod qq_tools;
pub mod screen_capture;
pub mod search_tools;
pub mod shell_tools;
pub mod uia_tools;
pub mod window_tools;

use lingxi_tools::ToolRegistry;
use std::sync::Arc;

/// Register all default Windows tools into a registry.
pub fn register_default_tools(registry: &mut ToolRegistry) {
    // Generic UIA selection/write tools are intentionally not registered yet:
    // while the Agent panel owns focus they would operate on the prompt box
    // rather than the user's previous application. They will be enabled after
    // target-window tracking is implemented. QQ has an explicit remembered
    // window and is therefore safe to expose below.

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

    // Window management (ROADMAP 0.3)
    registry.register(Arc::new(window_tools::ListWindowsTool));
    registry.register(Arc::new(window_tools::FocusWindowTool));
    registry.register(Arc::new(window_tools::CaptureScreenTool));

    // Application launch (ROADMAP 0.3)
    registry.register(Arc::new(app_tools::OpenAppTool));

    // Input simulation (ROADMAP 0.3)
    registry.register(Arc::new(input_tools::TypeTextTool));
    registry.register(Arc::new(input_tools::SendKeysTool));
    registry.register(Arc::new(input_tools::ClickAtTool));

    // File search (ROADMAP 0.3)
    registry.register(Arc::new(search_tools::SearchFilesTool));

    // Cross-platform information tools (ROADMAP 0.4)
    registry.register(Arc::new(lingxi_tools::builtin::time::GetTimeTool));
    registry.register(Arc::new(lingxi_tools::builtin::calc::CalculateTool));
    registry.register(Arc::new(lingxi_tools::builtin::fetch::WebFetchTool));
    registry.register(Arc::new(lingxi_tools::builtin::search::WebSearchTool));
    registry.register(Arc::new(lingxi_tools::builtin::translate::TranslateTool));
    registry.register(Arc::new(lingxi_tools::builtin::reminder::SetReminderTool));
    registry.register(Arc::new(lingxi_tools::builtin::reminder::ListRemindersTool));
    registry.register(Arc::new(
        lingxi_tools::builtin::reminder::CancelReminderTool,
    ));

    // High-impact tools stay unavailable until the UI has collected an
    // explicit per-invocation approval. This prevents a model from writing
    // files or executing commands merely because a global toggle was left on.
    registry.set_enabled("write_file", false);
    registry.set_enabled("run_command", false);
    registry.set_enabled("open_app", false);
}

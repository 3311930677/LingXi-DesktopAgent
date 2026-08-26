//! 桌宠皮肤库：扫描 `ui/assets/skins/<id>/skin.json` 皮肤包，
//! 提供清单解析、路径校验与列表查询。
//!
//! 皮肤包格式：
//! ```json
//! {
//!   "id": "lingxi-hamster",
//!   "name": "灵犀仓鼠",
//!   "author": "LingXi",
//!   "version": "1.0.0",
//!   "description": "…",
//!   "states": {
//!     "idle":      { "image": "idle.png",     "bubble": "灵犀" },
//!     "thinking":  { "image": "thinking.png", "bubble": "思考中…" },
//!     "speaking":  { "image": "speaking.png", "bubble": "建议好了" },
//!     "alert":     { "image": "alert.png",    "bubble": "QQ 新消息" }
//!   }
//! }
//! ```
//! 校验规则：id 只允许 `[A-Za-z0-9_-]`（防路径穿越）；四个状态图片必须存在；
//! 图片文件名不得包含路径分隔符。

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

pub const DEFAULT_SKIN_ID: &str = "lingxi-hamster";
pub const PET_STATES: [&str; 4] = ["idle", "thinking", "speaking", "alert"];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetSkinState {
    pub image: String,
    #[serde(default)]
    pub bubble: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PetSkinManifest {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub description: String,
    pub states: BTreeMap<String, PetSkinState>,
}

/// 设置页皮肤列表条目。
#[derive(Debug, Clone, Serialize)]
pub struct PetSkinInfo {
    pub id: String,
    pub name: String,
    pub author: String,
    pub version: String,
    pub description: String,
    /// 相对 `ui/` 的缩略图 URL（idle 态图片）。
    pub thumbnail: String,
}

/// 桌宠整体配置（发给桌宠窗口与设置页的完整视图）。
#[derive(Debug, Clone, Serialize)]
pub struct PetSkinView {
    pub skin: PetSkinManifest,
    /// 相对 `ui/` 的状态图 URL，键为状态名。
    pub images: BTreeMap<String, String>,
    /// 最终生效的气泡文案（用户覆盖优先，缺省回落皮肤默认），键为状态名。
    pub bubbles: BTreeMap<String, String>,
    /// 用户原始覆盖值（设置页回填输入框用；None = 未覆盖）。
    pub overrides: PetBubbleOverrides,
    pub visible: bool,
}

/// 用户气泡文案覆盖（空 = 使用皮肤默认文案）。
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PetBubbleOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speaking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alert: Option<String>,
}

impl PetBubbleOverrides {
    pub fn text_for(&self, state: &str) -> Option<&str> {
        match state {
            "idle" => self.idle.as_deref(),
            "thinking" => self.thinking.as_deref(),
            "speaking" => self.speaking.as_deref(),
            "alert" => self.alert.as_deref(),
            _ => None,
        }
    }
}

/// 皮肤 id 合法性：`[A-Za-z0-9_-]+`，防路径穿越。
pub fn valid_skin_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 64
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// 图片文件名合法性：非空且不含路径分隔符 / 上跳。
fn valid_image_name(name: &str) -> bool {
    !name.is_empty()
        && !name.contains('/')
        && !name.contains('\\')
        && !name.contains("..")
        && !name.contains(':')
}

/// 定位皮肤目录：从当前可执行文件向上找 `ui/assets/skins`。
/// 不依赖编译期路径，仓库整体移动后无需重编即可生效。
pub fn skins_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors()
        .skip(1)
        .map(|ancestor| ancestor.join("ui").join("assets").join("skins"))
        .find(|candidate| candidate.join(DEFAULT_SKIN_ID).join("skin.json").is_file())
}

/// 皮肤目录（id 已校验）。
pub fn skin_dir(id: &str) -> Option<PathBuf> {
    if !valid_skin_id(id) {
        return None;
    }
    skins_dir().map(|root| root.join(id))
}

/// 解析并校验皮肤清单。目录必须存在且四态图齐全。
pub fn load_manifest(id: &str) -> Result<PetSkinManifest, String> {
    let dir = skin_dir(id).ok_or_else(|| format!("皮肤目录不可用：{id}"))?;
    let manifest_path = dir.join("skin.json");
    let raw = std::fs::read_to_string(&manifest_path)
        .map_err(|error| format!("读取皮肤清单失败（{}）：{error}", manifest_path.display()))?;
    let manifest: PetSkinManifest =
        serde_json::from_str(&raw).map_err(|error| format!("皮肤清单格式错误（{id}）：{error}"))?;
    if manifest.id != id {
        return Err(format!(
            "皮肤目录与清单 id 不一致：目录 {id}，清单 {}",
            manifest.id
        ));
    }
    if manifest.name.trim().is_empty() {
        return Err(format!("皮肤 {id} 缺少 name"));
    }
    for state in PET_STATES {
        let entry = manifest
            .states
            .get(state)
            .ok_or_else(|| format!("皮肤 {id} 缺少 {state} 状态"))?;
        if !valid_image_name(&entry.image) {
            return Err(format!("皮肤 {id} 的 {state} 图片名非法：{}", entry.image));
        }
        if !dir.join(&entry.image).is_file() {
            return Err(format!(
                "皮肤 {id} 的 {state} 图片缺失：{}",
                entry.image
            ));
        }
    }
    Ok(manifest)
}

/// 列出全部有效皮肤（无效目录跳过并记日志，不影响其他皮肤）。
pub fn list_skins() -> Vec<PetSkinInfo> {
    let Some(root) = skins_dir() else {
        eprintln!("[lingxi] pet_skin: 未找到 ui/assets/skins 目录");
        return Vec::new();
    };
    let mut skins = Vec::new();
    let Ok(entries) = std::fs::read_dir(&root) else {
        return skins;
    };
    for entry in entries.flatten() {
        let Some(id) = entry.file_name().to_str().map(str::to_string) else {
            continue;
        };
        if !valid_skin_id(&id) || !entry.path().is_dir() {
            continue;
        }
        match load_manifest(&id) {
            Ok(manifest) => {
                let thumbnail = manifest
                    .states
                    .get("idle")
                    .map(|state| format!("assets/skins/{id}/{}", state.image))
                    .unwrap_or_default();
                skins.push(PetSkinInfo {
                    id: manifest.id,
                    name: manifest.name,
                    author: manifest.author,
                    version: manifest.version,
                    description: manifest.description,
                    thumbnail,
                });
            }
            Err(error) => eprintln!("[lingxi] pet_skin: 跳过无效皮肤 {id}：{error}"),
        }
    }
    skins.sort_by(|a, b| a.id.cmp(&b.id));
    skins
}

/// 构建发给前端的完整视图。指定的皮肤无效时回落默认皮肤。
pub fn view_for(
    skin_id: &str,
    overrides: &PetBubbleOverrides,
    visible: bool,
) -> Result<PetSkinView, String> {
    let manifest = load_manifest(skin_id)
        .or_else(|error| {
            eprintln!("[lingxi] pet_skin: 皮肤 {skin_id} 不可用（{error}），回落默认皮肤");
            load_manifest(DEFAULT_SKIN_ID)
        })
        .map_err(|error| format!("默认皮肤也不可用：{error}"))?;
    let images = manifest
        .states
        .iter()
        .map(|(state, entry)| {
            (
                state.clone(),
                format!("assets/skins/{}/{}", manifest.id, entry.image),
            )
        })
        .collect();
    let bubbles = manifest
        .states
        .iter()
        .map(|(state, entry)| {
            let text = overrides
                .text_for(state)
                .map(str::to_string)
                .unwrap_or_else(|| entry.bubble.clone());
            (state.clone(), text)
        })
        .collect();
    Ok(PetSkinView {
        skin: manifest,
        images,
        bubbles,
        overrides: overrides.clone(),
        visible,
    })
}

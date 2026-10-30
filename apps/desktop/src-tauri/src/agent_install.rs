//! 环境内一键安装 Agent：目录、探测、安装脚本和提示词模板。
//!
//! 目录与 `mixumOS/mixum-ctl agent`（#69）保持一致；安装通过环境里的一次
//! shell 任务完成，因此同样受快照/隔离/审计边界约束。

use std::time::Duration;

use mixum_env_core::{ContainerState, RuntimeKind};
use serde::Serialize;
use tauri::{AppHandle, State};

use crate::commands::{run_environment_script, run_map_script, CommandResult};
use crate::AppState;

pub struct AgentSpec {
    pub name: &'static str,
    pub summary: &'static str,
    /// exit 0 表示已安装。
    probe: &'static str,
    /// POSIX sh 安装脚本；`set -e` 由 install_script 加。
    install: &'static str,
}

const NPM_BOOTSTRAP: &str = r#"if ! command -v npm >/dev/null 2>&1; then
  if [ "$(id -u)" != "0" ]; then
    echo "缺少 npm；当前任务不是 root，无法用包管理器安装。请管理员预装 nodejs/npm。" >&2
    exit 1
  fi
  if command -v pacman >/dev/null 2>&1; then
    pacman -Sy --noconfirm nodejs npm
  elif command -v apk >/dev/null 2>&1; then
    apk add --no-cache nodejs npm
  elif command -v apt-get >/dev/null 2>&1; then
    apt-get update && apt-get install -y nodejs npm
  else
    echo "缺少 npm，且未识别包管理器" >&2
    exit 1
  fi
fi"#;

const CURL_BOOTSTRAP: &str = r#"if ! command -v curl >/dev/null 2>&1; then
  if [ "$(id -u)" != "0" ]; then
    echo "缺少 curl；当前任务不是 root，无法用包管理器安装。请管理员预装 curl。" >&2
    exit 1
  fi
  if command -v pacman >/dev/null 2>&1; then
    pacman -Sy --noconfirm curl
  elif command -v apk >/dev/null 2>&1; then
    apk add --no-cache curl
  elif command -v apt-get >/dev/null 2>&1; then
    apt-get update && apt-get install -y curl
  else
    echo "缺少 curl，且未识别包管理器" >&2
    exit 1
  fi
fi"#;

pub const AGENTS: &[AgentSpec] = &[
    AgentSpec {
        name: "claude-code",
        summary: "Anthropic Claude Code（npm 官方包）",
        probe: "claude --version >/dev/null 2>&1",
        install: r#"npm install -g --prefix "$HOME/.local" --allow-scripts=@anthropic-ai/claude-code @anthropic-ai/claude-code
claude --version >/dev/null"#,
    },
    AgentSpec {
        name: "codex",
        summary: "OpenAI Codex CLI（npm 官方包）",
        probe: "command -v codex >/dev/null 2>&1 || [ -x \"$HOME/.local/bin/codex\" ]",
        install: r#"npm install -g --prefix "$HOME/.local" @openai/codex
command -v codex || test -x "$HOME/.local/bin/codex""#,
    },
    AgentSpec {
        name: "gemini-cli",
        summary: "Google Gemini CLI（npm 官方包）",
        probe: "command -v gemini >/dev/null 2>&1 || [ -x \"$HOME/.local/bin/gemini\" ]",
        install: r#"npm install -g --prefix "$HOME/.local" --allow-scripts=@github/keytar,node-pty @google/gemini-cli
command -v gemini || test -x "$HOME/.local/bin/gemini""#,
    },
    AgentSpec {
        name: "opencode",
        summary: "OpenCode（官方安装脚本）",
        probe: "command -v opencode >/dev/null 2>&1 || [ -x \"$HOME/.opencode/bin/opencode\" ]",
        install: r#"curl -fsSL https://opencode.ai/install | bash
test -x "$HOME/.opencode/bin/opencode""#,
    },
];

pub fn find(name: &str) -> Option<&'static AgentSpec> {
    AGENTS.iter().find(|agent| agent.name == name)
}

pub fn template(name: &str) -> String {
    format!(
        r#"你正在 mixumOS 容器内工作（由 mixumOS 管理，agent 名称：{name}）。
工作区：/var/lib/mixum/work。任务开始时 mixumOS 会自动创建快照，长任务失败可回退重跑。
任务：{{{{instruction}}}}
要求：
- 先读代码和文档，再动手修改；一次只做一件事；
- 命令在容器内执行，网络与文件系统都限制在容器里；
- 不要修改 /var/lib/mixum、/run/mixum 下的运行时文件；
- 完成后给出改动清单与验证结果。"#
    )
}

fn prompt_write(spec: &AgentSpec) -> String {
    format!(
        "mkdir -p \"$HOME/.mixum/agents/{name}\"\ncat > \"$HOME/.mixum/agents/{name}/PROMPT.md\" <<'MIXUM_PROMPT_EOF'\n{template}\nMIXUM_PROMPT_EOF\n",
        name = spec.name,
        template = template(spec.name)
    )
}

pub fn install_script(spec: &AgentSpec) -> String {
    let mut script = String::from(
        "set -e\nexport PATH=\"$HOME/.local/bin:$HOME/.opencode/bin:$PATH\"\nmkdir -p \"$HOME/.local/bin\"\n",
    );
    if spec.name == "opencode" {
        script.push_str(CURL_BOOTSTRAP);
    } else {
        script.push_str(NPM_BOOTSTRAP);
    }
    script.push('\n');
    script.push_str(spec.install);
    script.push('\n');
    script.push_str(&prompt_write(spec));
    script
}

pub fn probe_script() -> String {
    let mut script = String::from(
        "export PATH=\"$HOME/.local/bin:$HOME/.opencode/bin:$PATH\"\n",
    );
    for spec in AGENTS {
        script.push_str(&format!(
            "if {}; then echo 'installed {}'; else echo 'missing {}'; fi\n",
            spec.probe, spec.name, spec.name
        ));
    }
    script
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentInfo {
    pub name: &'static str,
    pub summary: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentStatus {
    pub name: &'static str,
    pub summary: &'static str,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEnvironment {
    pub id: String,
    pub name: String,
    pub kind: RuntimeKind,
    pub addr: String,
}

#[tauri::command]
pub fn agent_catalog() -> Vec<AgentInfo> {
    AGENTS
        .iter()
        .map(|spec| AgentInfo {
            name: spec.name,
            summary: spec.summary,
        })
        .collect()
}

#[tauri::command]
pub fn agent_prompt(name: String) -> Result<String, String> {
    let spec = find(&name).ok_or_else(|| format!("未知 agent: {name}"))?;
    Ok(template(spec.name))
}

/// 列出正在运行、带 agentd 端口的 mixumOS 环境。
#[tauri::command]
pub async fn agent_environments(
    state: State<'_, AppState>,
) -> Result<Vec<AgentEnvironment>, String> {
    let mut environments = Vec::new();
    for backend in state.registry.statuses().await {
        if !backend.status.is_ready() {
            continue;
        }
        let Some(runtime) = state.registry.get(backend.kind) else {
            continue;
        };
        let Ok(containers) = runtime.list_containers().await else {
            continue;
        };
        for container in containers {
            if container.state != ContainerState::Running
                || !container.image.to_ascii_lowercase().contains("mixumos")
            {
                continue;
            }
            let port = container
                .ports
                .iter()
                .find(|port| port.container == 7601)
                .or_else(|| container.ports.first());
            let Some(port) = port else {
                continue;
            };
            environments.push(AgentEnvironment {
                id: container.id,
                name: container.name,
                kind: backend.kind,
                addr: format!("127.0.0.1:{}", port.host),
            });
        }
    }
    Ok(environments)
}

fn statuses_from(result: CommandResult) -> Result<Vec<AgentStatus>, String> {
    if result.state != "completed" {
        return Err(result
            .error
            .unwrap_or_else(|| format!("探测失败（{}）", result.state)));
    }
    Ok(AGENTS
        .iter()
        .map(|spec| AgentStatus {
            name: spec.name,
            summary: spec.summary,
            installed: result
                .lines
                .iter()
                .any(|line| line == &format!("installed {}", spec.name)),
        })
        .collect())
}

#[tauri::command]
pub async fn agent_status(
    app: AppHandle,
    container_id: String,
) -> Result<Vec<AgentStatus>, String> {
    let result = run_environment_script(
        &app,
        &container_id,
        &probe_script(),
        Duration::from_secs(120),
        None,
    )
    .await?;
    statuses_from(result)
}

#[tauri::command]
pub async fn agent_install(
    app: AppHandle,
    container_id: String,
    name: String,
    install_id: Option<String>,
) -> Result<CommandResult, String> {
    let spec = find(&name).ok_or_else(|| format!("未知 agent: {name}"))?;
    run_environment_script(
        &app,
        &container_id,
        &install_script(spec),
        Duration::from_secs(1800),
        install_id.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn agent_remote_status(
    app: AppHandle,
    addr: String,
    token: String,
    fingerprint: Option<String>,
    node: String,
) -> Result<Vec<AgentStatus>, String> {
    let result = run_map_script(
        &app,
        &addr,
        &token,
        fingerprint.as_deref(),
        Some(&node),
        &probe_script(),
        Duration::from_secs(120),
        None,
        None,
    )
    .await?;
    statuses_from(result)
}

#[tauri::command]
pub async fn agent_remote_install(
    app: AppHandle,
    addr: String,
    token: String,
    fingerprint: Option<String>,
    node: String,
    name: String,
    install_id: Option<String>,
) -> Result<CommandResult, String> {
    let spec = find(&name).ok_or_else(|| format!("未知 agent: {name}"))?;
    run_map_script(
        &app,
        &addr,
        &token,
        fingerprint.as_deref(),
        Some(&node),
        &install_script(spec),
        Duration::from_secs(1800),
        None,
        install_id.as_deref(),
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_unique_and_known() {
        let names: Vec<_> = AGENTS.iter().map(|agent| agent.name).collect();
        for name in ["claude-code", "codex", "gemini-cli", "opencode"] {
            assert!(names.contains(&name), "缺少 {name}");
            assert!(find(name).is_some());
        }
        let mut sorted = names.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), names.len(), "agent 名称重复");
    }

    #[test]
    fn template_exposes_instruction_placeholder() {
        let text = template("codex");
        assert!(text.contains("{{instruction}}"));
        assert!(text.contains("codex"));
        assert!(text.contains("/var/lib/mixum/work"));
    }

    #[test]
    fn install_script_writes_prompt_and_installs() {
        let script = install_script(find("claude-code").expect("claude-code"));
        assert!(script.starts_with("set -e\n"));
        assert!(script.contains(
            "npm install -g --prefix \"$HOME/.local\" --allow-scripts=@anthropic-ai/claude-code @anthropic-ai/claude-code"
        ));
        assert!(script.contains("claude --version >/dev/null"));
        let gemini = install_script(find("gemini-cli").expect("gemini-cli"));
        assert!(gemini.contains("--allow-scripts=@github/keytar,node-pty"));
        assert!(script.contains("$HOME/.mixum/agents/claude-code/PROMPT.md"));
        assert!(script.contains("MIXUM_PROMPT_EOF"));
        let opencode = install_script(find("opencode").expect("opencode"));
        assert!(opencode.contains("curl -fsSL https://opencode.ai/install"));
        assert!(!opencode.contains("@anthropic-ai/claude-code"));
    }

    #[test]
    fn probe_script_covers_every_agent() {
        let script = probe_script();
        for spec in AGENTS {
            assert!(script.contains(&format!("installed {}", spec.name)));
            assert!(script.contains(&format!("missing {}", spec.name)));
        }
    }
}

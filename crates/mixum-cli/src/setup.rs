use std::io::{self, IsTerminal, Write};
use std::path::Path;

use mixum_env_core::RuntimeKind;
use mixum_setup::{Language, SetupPlan, SetupRequest};

use crate::cli::{RuntimeArg, SetupArgs};
use crate::env;
use crate::output::{self, CliError, Result};

fn tr(language: Language, zh: &'static str, en: &'static str) -> &'static str {
    match language {
        Language::ZhCn => zh,
        Language::En => en,
    }
}

fn parse_language(value: &str) -> Result<Language> {
    match value.to_ascii_lowercase().as_str() {
        "zh" | "zh-cn" | "zh_cn" | "cn" => Ok(Language::ZhCn),
        "en" | "en-us" | "en_us" => Ok(Language::En),
        _ => Err(CliError::usage("--lang must be zh-CN or en")),
    }
}

fn tty() -> bool {
    io::stdin().is_terminal() && io::stdout().is_terminal()
}

fn ask(prompt: &str) -> Result<String> {
    print!("{prompt}");
    io::stdout()
        .flush()
        .map_err(|error| CliError::io(format!("cannot write prompt: {error}")))?;
    let mut line = String::new();
    io::stdin()
        .read_line(&mut line)
        .map_err(|error| CliError::io(format!("cannot read input: {error}")))?;
    Ok(line.trim().to_string())
}

fn confirm(prompt: &str, yes: bool) -> Result<bool> {
    if yes {
        return Ok(true);
    }
    if !tty() {
        return Err(CliError::usage(format!(
            "{prompt}: cannot prompt without a TTY; pass --yes"
        )));
    }
    let answer = ask(&format!("{prompt} [y/N] "))?;
    Ok(matches!(answer.to_ascii_lowercase().as_str(), "y" | "yes"))
}

fn status_text(status: &mixum_env_core::RuntimeStatus) -> String {
    match status {
        mixum_env_core::RuntimeStatus::Ready { version, detail } => {
            let mut text = "ready".to_string();
            if let Some(version) = version {
                text.push_str(&format!(" ({version})"));
            }
            if let Some(detail) = detail {
                text.push_str(&format!(" — {detail}"));
            }
            text
        }
        mixum_env_core::RuntimeStatus::NotInstalled { hint }
        | mixum_env_core::RuntimeStatus::NotRunning { hint }
        | mixum_env_core::RuntimeStatus::Unsupported { reason: hint } => hint.clone(),
    }
}

fn default_runner(plan: &SetupPlan) -> Option<RuntimeKind> {
    plan.runners
        .iter()
        .find(|runner| runner.supported && runner.status.is_ready())
        .map(|runner| runner.kind)
        .or_else(|| {
            plan.runners
                .iter()
                .find(|runner| runner.supported && runner.install.is_some())
                .map(|runner| runner.kind)
        })
}

fn choose_runner(
    plan: &SetupPlan,
    requested: Option<RuntimeKind>,
    language: Language,
) -> Result<RuntimeKind> {
    if let Some(kind) = requested {
        return Ok(kind);
    }
    let supported: Vec<_> = plan
        .runners
        .iter()
        .filter(|runner| runner.supported)
        .collect();
    if supported.is_empty() {
        return Err(CliError::runtime(tr(
            language,
            "当前平台没有可用的 VM Runner。",
            "No VM runner is available on this platform.",
        )));
    }
    if let Some(kind) = default_runner(plan) {
        if !tty() {
            return Ok(kind);
        }
        println!(
            "\n{}",
            tr(language, "选择 VM Runner：", "Choose a VM runner:")
        );
        for (index, runner) in supported.iter().enumerate() {
            println!(
                "  {}. {} — {}{}",
                index + 1,
                runner.name,
                status_text(&runner.status),
                if runner.kind == kind {
                    tr(language, " (默认)", " (default)")
                } else {
                    ""
                }
            );
        }
        let answer = ask(tr(
            language,
            "编号 [回车默认]：",
            "Number [Enter for default]: ",
        ))?;
        if answer.is_empty() {
            return Ok(kind);
        }
        let index = answer
            .parse::<usize>()
            .ok()
            .and_then(|value| value.checked_sub(1))
            .ok_or_else(|| CliError::usage("invalid runner number"))?;
        return supported
            .get(index)
            .map(|runner| runner.kind)
            .ok_or_else(|| CliError::usage("runner number out of range"));
    }
    Err(CliError::runtime(tr(
        language,
        "没有可用的 VM Runner。请先安装 Docker、Apple Container 或 WSL 2。",
        "No VM runner is ready. Install Docker, Apple Container or WSL 2 first.",
    )))
}

async fn ensure_runner(
    registry: &mixum_env_core::BackendRegistry,
    plan: &mut SetupPlan,
    kind: RuntimeKind,
    yes: bool,
    language: Language,
    data_dir: &Path,
) -> Result<()> {
    let Some(runner) = plan.runners.iter().find(|runner| runner.kind == kind) else {
        return Err(CliError::usage("unknown runner"));
    };
    if runner.status.is_ready() {
        return Ok(());
    }
    if !runner.supported {
        return Err(CliError::runtime(format!(
            "{} {}",
            runner.name,
            tr(
                language,
                "在当前平台不可用。",
                "is not available on this platform."
            )
        )));
    }
    let (action, installing) = if matches!(
        runner.status,
        mixum_env_core::RuntimeStatus::NotRunning { .. }
    ) {
        let Some(action) = runner.start.clone() else {
            return Err(CliError::runtime(status_text(&runner.status)));
        };
        (action, false)
    } else {
        let Some(action) = runner.install.clone() else {
            return Err(CliError::runtime(status_text(&runner.status)));
        };
        (action, true)
    };
    println!(
        "\n{} {}\n  $ {}\n  {}",
        tr(
            language,
            if installing {
                "需要安装"
            } else {
                "需要启动"
            },
            if installing {
                "Install required:"
            } else {
                "Start required:"
            }
        ),
        runner.name,
        action.command.join(" "),
        action.note
    );
    if !confirm(
        tr(
            language,
            if installing {
                "现在执行安装？"
            } else {
                "现在启动？"
            },
            if installing {
                "Run the installer now?"
            } else {
                "Start it now?"
            },
        ),
        yes,
    )? {
        return Err(CliError::runtime(tr(
            language,
            "VM Runner 未就绪，setup 已取消。",
            "VM runner is not ready; setup cancelled.",
        )));
    }
    let output = if installing {
        mixum_setup::install_runner(kind).await
    } else {
        mixum_setup::start_runner(kind).await
    }
    .map_err(|error| CliError::runtime(error.to_string()))?;
    if !output.trim().is_empty() {
        println!("{output}");
    }
    // Give a just-started daemon a short grace period, then re-probe.
    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    *plan = mixum_setup::plan(registry, data_dir, &plan.launcher)
        .await
        .map_err(|error| CliError::runtime(error.to_string()))?;
    let ready = plan
        .runners
        .iter()
        .find(|runner| runner.kind == kind)
        .is_some_and(|runner| runner.status.is_ready());
    if !ready {
        return Err(CliError::runtime(tr(
            language,
            "安装命令已结束，但 Runner 仍未就绪；请按上方提示启动它后重新运行 mixum setup。",
            "The installer finished but the runner is not ready; start it as described and rerun mixum setup.",
        )));
    }
    Ok(())
}

fn choose_image(
    plan: &SetupPlan,
    requested: Option<&str>,
    language: Language,
) -> Result<Option<String>> {
    if let Some(image) = requested {
        return Ok(Some(image.to_string()));
    }
    let mut images: Vec<_> = plan
        .images
        .iter()
        .filter(|image| image.reference.to_ascii_lowercase().contains("mixumos"))
        .collect();
    if images.is_empty() {
        return Ok(None);
    }
    if !tty() {
        return Ok(Some(images.remove(0).reference.clone()));
    }
    println!(
        "\n{}",
        tr(language, "选择 mixumOS 镜像：", "Choose a mixumOS image:")
    );
    for (index, image) in images.iter().enumerate() {
        println!("  {}. {} ({})", index + 1, image.reference, image.runtime);
    }
    let answer = ask(tr(
        language,
        "编号 [回车使用第一个]：",
        "Number [Enter for first]: ",
    ))?;
    if answer.is_empty() {
        return Ok(Some(images[0].reference.clone()));
    }
    let index = answer
        .parse::<usize>()
        .ok()
        .and_then(|value| value.checked_sub(1))
        .ok_or_else(|| CliError::usage("invalid image number"))?;
    images
        .get(index)
        .map(|image| Some(image.reference.clone()))
        .ok_or_else(|| CliError::usage("image number out of range"))
}

fn choose_agents(plan: &SetupPlan, args: &SetupArgs, language: Language) -> Result<Vec<String>> {
    if !args.agents.is_empty() {
        for id in &args.agents {
            if !plan.clients.iter().any(|client| client.id == *id) {
                return Err(CliError::usage(format!("unknown agent client: {id}")));
            }
        }
        return Ok(args.agents.clone());
    }
    let defaults: Vec<_> = plan
        .clients
        .iter()
        .filter(|client| client.status == "configured" || client.status == "needs_update")
        .map(|client| client.id.clone())
        .collect();
    if args.all_detected {
        return Ok(plan
            .clients
            .iter()
            .filter(|client| client.status != "missing")
            .map(|client| client.id.clone())
            .collect());
    }
    if !tty() {
        return Ok(defaults);
    }
    println!(
        "\n{}",
        tr(
            language,
            "安装 MCP 到哪些 Agent：",
            "Install MCP into which agents:"
        )
    );
    for (index, client) in plan.clients.iter().enumerate() {
        println!(
            "  {}. {} — {}{}",
            index + 1,
            client.name,
            client.status,
            if defaults.contains(&client.id) {
                tr(language, " [默认]", " [default]")
            } else {
                ""
            }
        );
    }
    let answer = ask(tr(
        language,
        "编号，逗号分隔；回车使用默认：",
        "Numbers, comma-separated; Enter for defaults: ",
    ))?;
    if answer.is_empty() {
        return Ok(defaults);
    }
    answer
        .split(',')
        .map(|value| {
            let index = value
                .trim()
                .parse::<usize>()
                .ok()
                .and_then(|value| value.checked_sub(1))
                .ok_or_else(|| CliError::usage("invalid agent number"))?;
            plan.clients
                .get(index)
                .map(|client| client.id.clone())
                .ok_or_else(|| CliError::usage("agent number out of range"))
        })
        .collect()
}

pub async fn run(args: SetupArgs, data_dir: &Path, json: bool) -> Result<()> {
    let language = match args.lang.as_deref() {
        Some(value) => parse_language(value)?,
        None => {
            let saved = mixum_setup::load_settings(data_dir)
                .map_err(|error| CliError::io(error.to_string()))?
                .language;
            match saved {
                Some(language) => language,
                None if tty() => {
                    println!("Language / 语言:\n  1. 中文\n  2. English");
                    let answer = ask("选择语言 / Choose [1/2, Enter = 中文]: ")?;
                    if answer == "2" {
                        Language::En
                    } else {
                        Language::ZhCn
                    }
                }
                None => mixum_setup::detect_language(),
            }
        }
    };
    let launcher = std::env::current_exe()
        .map_err(|error| CliError::io(format!("cannot find mixum executable: {error}")))?
        .display()
        .to_string();
    let registry = env::registry();
    let mut plan = mixum_setup::plan(&registry, data_dir, &launcher)
        .await
        .map_err(|error| CliError::runtime(error.to_string()))?;
    let environment = match args.environment.as_deref() {
        Some(id) => {
            let environment = plan
                .environments
                .iter()
                .find(|environment| environment.id == id || environment.name == id)
                .ok_or_else(|| CliError::usage(format!("unknown environment: {id}")))?;
            Some((environment.runtime, environment.image.clone()))
        }
        None => None,
    };
    let requested = args
        .runtime
        .map(RuntimeArg::kind)
        .or_else(|| environment.as_ref().map(|(runtime, _)| *runtime));
    let runner = choose_runner(&plan, requested, language)?;
    if !args.dry_run {
        ensure_runner(&registry, &mut plan, runner, args.yes, language, data_dir).await?;
    }
    let environment_id = args.environment.clone();
    let selected_runtime = environment
        .as_ref()
        .map(|(runtime, _)| *runtime)
        .unwrap_or(runner);
    let mut image = match environment {
        Some((_, image)) => Some(image),
        None => choose_image(&plan, args.image.as_deref(), language)?,
    };
    let mut pull = args.pull;
    if environment_id.is_some() {
        pull = false;
    }
    if image.is_none() && tty() && !args.dry_run {
        println!(
            "\n{} {}",
            tr(
                language,
                "没有本地 mixumOS 镜像，默认拉取：",
                "No local mixumOS image; default pull:"
            ),
            mixum_setup::DEFAULT_IMAGE
        );
        if confirm(tr(language, "现在拉取？", "Pull it now?"), args.yes)? {
            image = Some(mixum_setup::DEFAULT_IMAGE.to_string());
            pull = true;
        }
    } else if image.is_none() && args.pull {
        image = Some(mixum_setup::DEFAULT_IMAGE.to_string());
    }
    if let Some(reference) = image.as_deref() {
        let local = plan
            .images
            .iter()
            .any(|choice| choice.reference == reference);
        if !local && !pull {
            if tty()
                && confirm(
                    tr(
                        language,
                        "该镜像不在本地，是否拉取？",
                        "This image is not local; pull it?",
                    ),
                    args.yes,
                )?
            {
                pull = true;
            } else if !args.dry_run {
                return Err(CliError::usage(
                    "image is not local; pass --pull to fetch it",
                ));
            }
        }
    }
    if image.is_none() && !args.dry_run {
        return Err(CliError::usage(
            "no mixumOS image selected; pass --image or --pull",
        ));
    }
    let agents = choose_agents(&plan, &args, language)?;
    if args.dry_run {
        if json {
            output::print_json(&serde_json::json!({
                "plan": plan,
                "selection": {
                    "language": language,
                    "runtime": selected_runtime,
                    "image": image,
                    "agents": agents,
                }
            }));
        } else {
            println!(
                "\n{}: {} / {} / {} / {}",
                tr(language, "计划", "Plan"),
                language.label(),
                selected_runtime.display_name(),
                image.as_deref().unwrap_or("-"),
                if agents.is_empty() {
                    "-".to_string()
                } else {
                    agents.join(", ")
                }
            );
        }
        return Ok(());
    }
    let request = SetupRequest {
        language,
        runtime: selected_runtime,
        image,
        pull_image: pull,
        environment_id,
        clients: agents,
    };
    let result = mixum_setup::apply(&registry, data_dir, &launcher, request)
        .await
        .map_err(|error| CliError::runtime(error.to_string()))?;
    if json {
        output::print_json(&result);
    } else {
        println!(
            "\n{} {} / {}",
            tr(language, "完成", "Done"),
            result.runner.display_name(),
            result.image.as_deref().unwrap_or("-")
        );
        if result.configured.is_empty() {
            println!(
                "{}",
                tr(language, "没有配置 Agent。", "No agent was configured.")
            );
        } else {
            println!(
                "{} {}",
                tr(language, "已配置 Agent：", "Configured agents:"),
                result.configured.join(", ")
            );
        }
        for failure in &result.failed {
            eprintln!(
                "{} {}: {}",
                tr(language, "失败", "failed"),
                failure.id,
                failure.error
            );
        }
        println!(
            "{}",
            tr(
                language,
                "重启对应 Agent 后生效。之后可随时运行 `mixum setup` 追加 Agent。",
                "Restart the affected agents. Rerun `mixum setup` anytime to add agents."
            )
        );
    }
    Ok(())
}

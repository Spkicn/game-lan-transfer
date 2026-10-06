//! 子进程调用
//!
//! 桌面程序没有控制台，每次 spawn 都会闪一个黑框；统一从这里构造命令避免闪烁

use std::process::Command;

/// Windows 上不创建控制台窗口的标志
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// 构造一个不会弹出控制台窗口的命令
#[must_use]
pub fn hidden_command(program: &str) -> Command {
    let mut command = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    command
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn hidden_command_runs_without_a_console() {
        // 没有控制台窗口也能正常拿到输出
        let output = hidden_command("cmd")
            .args(["/C", "echo", "ok"])
            .output()
            .expect("run");
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stdout).contains("ok"));
    }
}

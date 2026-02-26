use std::process::Command;

pub fn execute(command: &[String]) -> Result<(), String> {
    let mut cmd = Command::new(&command[0]);

    for arg in &command[1..] {
        cmd.arg(arg);
    }

    let status = cmd.status().map_err(|e| e.to_string())?;

    if !status.success() {
        return Err(format!("Command failed: {:?}", command));
    }

    Ok(())
}
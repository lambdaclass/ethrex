use crate::cli::EthrexL2CLI;
use clap::{CommandFactory, Subcommand, ValueEnum};
use clap_complete::{aot::Shell, generate};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, Write};

#[derive(Subcommand)]
pub(crate) enum Command {
    #[clap(about = "Generate autocomplete shell script.")]
    Generate {
        #[clap(short = 's', long = "shell", help = "Default: $SHELL")]
        shell: Option<Shell>,
    },
    #[clap(about = "Generate and install autocomplete shell script.")]
    Install {
        #[clap(short = 's', long = "shell", help = "Default: $SHELL")]
        shell: Option<Shell>,
    },
}

impl Command {
    pub fn run(self) -> eyre::Result<()> {
        match self {
            Command::Generate { shell } => generate_bash_script(shell),
            Command::Install { shell } => install_bash_script(shell),
        }
    }
}

fn get_shellrc_path(shell: Shell) -> eyre::Result<String> {
    match shell {
        Shell::Bash => Ok(".bashrc".to_owned()),
        Shell::Zsh => Ok(".zshrc".to_owned()),
        Shell::Fish => Ok(".config/fish/config.fish".to_owned()),
        Shell::Elvish => Ok(".elvish/rc.elv".to_owned()),
        Shell::PowerShell => Ok(".config/powershell/Microsoft.PowerShell_profile.ps1".to_owned()),
        _ => Err(eyre::eyre!(
            "Your shell is not supported. Supported shells are: {:?}",
            Shell::value_variants()
        )),
    }
}

fn get_shell(arg: Option<Shell>) -> eyre::Result<Shell> {
    if let Some(shell) = arg {
        Ok(shell)
    } else if let Some(env_shell) = Shell::from_env() {
        Ok(env_shell)
    } else {
        Err(eyre::eyre!(
            "Your shell is not supported. Supported shells are: {:?}",
            Shell::value_variants()
        ))
    }
}

fn generate_bash_script(shell_arg: Option<Shell>) -> eyre::Result<()> {
    let shell = get_shell(shell_arg)?;
    generate(
        shell,
        &mut EthrexL2CLI::command(),
        "ethrex_l2",
        &mut io::stdout(),
    );
    Ok(())
}

fn shellrc_command_exists(shellrc_path: &std::path::Path, shell: Shell) -> eyre::Result<bool> {
    let expected_string = if shell == Shell::Elvish {
        "-source $HOME/.ethrex-l2-completion"
    } else {
        ". $HOME/.ethrex-l2-completion"
    };

    let file = File::open(shellrc_path)?;
    let reader = io::BufReader::new(file);
    let lines = reader.lines();
    for line in lines {
        let line = line?;
        if line == expected_string {
            return Ok(true);
        }
    }

    Ok(false)
}

fn install_bash_script(shell_arg: Option<Shell>) -> eyre::Result<()> {
    let shell = get_shell(shell_arg)?;

    let file_path = dirs::home_dir()
        .ok_or(eyre::eyre!("Cannot find home directory."))?
        .join(".ethrex-l2-completion");
    let mut file = File::create(file_path)?;
    generate(shell, &mut EthrexL2CLI::command(), "ethrex_l2", &mut file);
    file.flush()?;

    let shellrc_path = dirs::home_dir()
        .ok_or(eyre::eyre!("Cannot find home directory."))?
        .join(get_shellrc_path(shell)?);

    if !shellrc_command_exists(&shellrc_path, shell)? {
        let mut file = OpenOptions::new().append(true).open(shellrc_path)?;
        if shell == Shell::Elvish {
            file.write_all(b"\n-source $HOME/.ethrex-l2-completion\n")?;
        } else {
            file.write_all(b"\n. $HOME/.ethrex-l2-completion\n")?;
        }
        file.flush()?;
    }

    println!("Autocomplete script installed. To apply changes, restart your shell.");
    Ok(())
}

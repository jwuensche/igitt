use async_std::io;
use std::fs::File;
use std::collections::BTreeMap as Map;
use anyhow::{bail, Result, Context};
use clap::{Arg, App};
use serde::{Serialize, Deserialize};
use serde_json::Value;
use regex::Regex;

#[derive(Serialize, Deserialize, Debug)]
struct Commit {
    origin: String,
    commit: String,
}

#[async_std::main]
async fn main() -> Result<()> {
    let matches = App::new("MSR Commit Viewer")
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .arg(Arg::with_name("KEYWORDS_YAML")
             .help("Sets the path to the keywords yaml file")
             .required(true))
        .arg(Arg::with_name("github-token")
             .help("Sets the GitHub API Token")
             .long("github")
             .value_name("TOKEN")
             .required(true))
        .arg(Arg::with_name("gitlab-token")
             .help("Sets the GitLab API Token")
             .long("gitlab")
             .value_name("TOKEN")
             .required(true))
        .get_matches();

    let keywords: Map<String, Vec<Commit>> = serde_yaml::from_reader(
        File::open(matches.value_of("KEYWORDS_YAML").context("KEYWORDS_YAML not provided")?)?
    )?;
    let github_token = matches.value_of("github-token").context("github-token not defined")?;
    let gitlab_token = matches.value_of("gitlab-token").context("gitlab-token not defined")?;

    let url_re = Regex::new(r"https://(.+?)/(.+)")?;
    let mut line = String::new();
    let stdin = io::stdin();

    for (kw, commits) in &keywords {
        for commit in commits {
            let captures = url_re.captures(&commit.origin).context("could not parse origin")?;
            let domain = captures.get(1).context("no valid domain for origin")?.as_str();
            let path = captures.get(2).context("no valid path for origin")?.as_str();

            let (msg_url, diff_url, auth) = match domain {
                "github.com" => (
                    format!("https://api.github.com/repos/{}/git/commits/{}", path, commit.commit),
                    format!("https://github.com/{}/commit/{}.diff", path, commit.commit),
                    ("Authorization", format!("token {}", github_token)),
                ),
                "gitlab.com" => (
                    format!("https://gitlab.com/api/v4/projects/{}/repository/commits/{}", path, commit.commit),
                    format!("https://gitlab.com/api/v4/projects/{}/repository/commits/{}/diff", path, commit.commit),
                    ("PRIVATE-TOKEN", gitlab_token.to_string()),
                ),
                d => bail!("invalid domain {}", d)
            };

            let message = match surf::get(msg_url).set_header(auth.0, auth.1.clone()).recv_json::<Value>().await {
                Ok(response) => response["message"].clone(),
                Err(err) => bail!(err),
            };
            let diff = match surf::get(diff_url).set_header(auth.0, auth.1).recv_string().await {
                Ok(response) => response,
                Err(err) => bail!(err),
            };

            println!("{}", message);
            println!("{}", diff);

            stdin.read_line(&mut line).await?;
        }
    }

    println!("keywords: {:#?}", keywords);
    Ok(())
}

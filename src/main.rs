use cursive::Cursive;
use cursive::align::HAlign;
use cursive::view::{Scrollable, Nameable, Resizable};
use cursive::views::{LinearLayout, TextView, EditView, Panel, Dialog};
use cursive_async_view::{AsyncView, AsyncState};
use async_std::prelude::*;
use async_std::task;
use std::sync::mpsc::channel;
use std::fs::File;
use std::thread;
use std::time::Duration;
use std::collections::BTreeMap as Map;
use anyhow::{bail, Result, Context};
use clap::{Arg, App};
use serde::{Serialize, Deserialize};
use serde_json::Value;
use regex::Regex;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};

#[derive(Serialize, Deserialize, Debug)]
struct Commit {
    origin: String,
    commit: String,
}

fn disable_btns(siv: &mut Cursive) {
    let _btns = siv.find_name::<Dialog>("keywords_dialog")
        .unwrap()
        .buttons_mut()
        .map(|btn| btn.disable())
        .collect::<Vec<_>>();
}

fn enable_btns(siv: &mut Cursive) {
    let _btns = siv.find_name::<Dialog>("keywords_dialog")
        .unwrap()
        .buttons_mut()
        .map(|btn| btn.enable())
        .collect::<Vec<_>>();
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

    let url_re = Regex::new(r"^https://(.+?)/(.+?)(:?\.git)?$")?;

    let (cb_sink_tx, cb_sink_rx) = channel();
    let (name_tx, name_rx) = channel();
    let siv_task_handle = task::spawn(async move {
        let mut siv = Cursive::default();
        cb_sink_tx.send(siv.cb_sink().clone()).unwrap();

        let name_edit_tx = name_tx.clone();
        let name_edit = EditView::new()
            .on_submit(move |siv, name| {
                if !name.is_empty() {
                    name_edit_tx.send(name.to_string()).unwrap();
                    siv.pop_layer();
                }
            })
            .with_name("name_text_field");
        let name_dialog = Dialog::around(name_edit)
            .title("Please enter your name")
            .button("Ok", move |siv| {
                let name = siv.find_name::<EditView>("name_text_field")
                    .unwrap()
                    .get_content()
                    .to_string();

                if !name.is_empty() {
                    name_tx.send(name).unwrap();
                    siv.pop_layer();
                }
            });

        siv.add_layer(name_dialog);
        siv.run();
    });
    let cb_sink = cb_sink_rx.recv().unwrap();
    let name = name_rx.recv().unwrap();

    let (next_tx, next_rx) = channel();
    let (quit_tx, quit_rx) = channel();
    cb_sink.send(Box::new(move |mut siv| {
        let keywords_dialog = Dialog::new()
            .button("Next", move |mut siv| {
                disable_btns(&mut siv);
                next_tx.send(()).unwrap();
            })
            .with_name("keywords_dialog")
            .full_screen();

        siv.add_layer(keywords_dialog);
        disable_btns(&mut siv);

        siv.add_global_callback('q', move |siv| {
            let quit_tx_cp = quit_tx.clone();
            siv.add_layer(
                Dialog::text("Do you really want to quit and discard all changes?")
                    .button("Yes", move |_siv| quit_tx_cp.send(()).unwrap())
                    .button("No", move |siv| { siv.pop_layer(); })
            );
        });
    })).unwrap();

    'outer: for (kw, commits) in &keywords {
        for commit in commits {
            let captures = url_re.captures(&commit.origin).context("could not parse origin")?;
            let domain = captures.get(1).context("no valid domain for origin")?.as_str();
            let path = captures.get(2).context("no valid path for origin")?.as_str();
            let urlenc = utf8_percent_encode(path, NON_ALPHANUMERIC);

            let (msg_url, diff_url, auth) = match domain {
                "github.com" => (
                    format!("https://api.github.com/repos/{}/git/commits/{}", path, commit.commit),
                    format!("https://github.com/{}/commit/{}.diff", path, commit.commit),
                    ("Authorization", format!("token {}", github_token)),
                ),
                "gitlab.com" => (
                    format!("https://gitlab.com/api/v4/projects/{}/repository/commits/{}", urlenc, commit.commit),
                    format!("https://gitlab.com/{}/-/commit/{}.diff", path, commit.commit),
                    ("PRIVATE-TOKEN", gitlab_token.to_string()),
                ),
                d => bail!("invalid domain {}", d)
            };

            let message_request = surf::get(msg_url).set_header(auth.0, auth.1.clone()).recv_json::<Value>();
            let diff_request = surf::get(diff_url).set_header(auth.0, auth.1).recv_string();

            let (tx, rx) = channel();
            async_std::task::spawn(async move {
                let result = message_request.join(diff_request).await;
                tx.send(result).expect("sending over channel failed");
            });

            let keyword = kw.clone();
            let origin = commit.origin.clone();
            let commithash = commit.commit.clone();
            let inner_cb_sink = cb_sink.clone();

            cb_sink.send(Box::new(move |mut siv| {
                let keyword = keyword.clone();
                let origin = origin.clone();
                let commithash = commithash.clone();

                let mut keywords_dialog = siv.find_name::<Dialog>("keywords_dialog").unwrap();
                keywords_dialog.set_title(format!(
                    "Loading '{keyword}' / {section} | {origin} @ {commit} - {date}",
                    keyword = keyword,
                    section = "N/A",
                    origin = origin,
                    date = "N/A",
                    commit = commithash,
                ));

                let async_view = AsyncView::new(&mut siv, move || {
                    let (message_result, diff_result) = match rx.try_recv() {
                        Ok(req) => req,
                        Err(_) => return AsyncState::Pending,
                    };

                    let mut linear = LinearLayout::vertical();

                    let message = match message_result {
                        Ok(message) => message["message"].as_str()
                            .unwrap_or("!! Commit message not available !!")
                            .to_string(),
                        Err(err) => format!("{}", err),
                    };

                    linear.add_child(
                        Panel::new(TextView::new(message).scrollable().scroll_x(false).scroll_y(true))
                            .title("Commit Message")
                            .title_position(HAlign::Left)
                    );

                    let diff = match diff_result {
                        Ok(diff) => diff,
                        Err(err) => format!("{}", err),
                    };

                    linear.add_child(
                        Panel::new(TextView::new(diff).scrollable().scroll_x(false).scroll_y(true))
                            .title("Diff")
                            .title_position(HAlign::Left)
                            .full_screen()
                    );

                    let keyword = keyword.clone();
                    let origin = origin.clone();
                    let commithash = commithash.clone();
                    inner_cb_sink.send(Box::new(move |mut siv| {
                        enable_btns(&mut siv);
                        let mut keywords_dialog = siv.find_name::<Dialog>("keywords_dialog").unwrap();
                        keywords_dialog.set_title(format!(
                            "'{keyword}' / {section} | {origin} @ {commit} - {date}",
                            keyword = keyword,
                            origin = origin,
                            section = "N/A",
                            commit = commithash,
                            date = "N/A",
                        ));
                    })).unwrap();

                    AsyncState::Available(linear)
                });

                keywords_dialog.set_content(async_view);
            })).unwrap();

            loop {
                match quit_rx.try_recv() {
                    Ok(_) => break 'outer,
                    Err(_) => {},
                }

                match next_rx.try_recv() {
                    Ok(_) => break,
                    Err(_) => thread::sleep(Duration::from_millis(10)),
                }
            }
        }
    }

    cb_sink.send(Box::new(move |siv| siv.quit())).unwrap();
    siv_task_handle.await;

    Ok(())
}

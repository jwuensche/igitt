use anyhow::{bail, Context, Result};
use async_std::prelude::*;
use async_std::task;
use clap::{App, Arg};
use cursive::align::HAlign;
use cursive::view::{Nameable, Resizable, Scrollable};
use cursive::views::{
    Button, Dialog, DummyView, EditView, LinearLayout, Panel, RadioButton, RadioGroup, SelectView,
    TextArea, TextView,
};
use cursive::Cursive;
use cursive_aligned_view::Alignable;
use cursive_async_view::{AsyncState, AsyncView};
use cursive_tabs::TabPanel;
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap as Map;
use std::collections::HashSet;
use std::fs::File;
use std::sync::mpsc::channel;
use std::thread;
use std::time::Duration;

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Rating {
    is_refactoring: bool,
    comment: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
struct Commit {
    origin: String,
    commit: String,
    #[serde(default)]
    rating: Map<String, Rating>,
}

enum Paging {
    Next(String, bool),
    Prev(String, bool),
    Finish(String, bool),
}

#[async_std::main]
async fn main() -> Result<()> {
    let matches = App::new("MSR Commit Viewer")
        .version(env!("CARGO_PKG_VERSION"))
        .author(env!("CARGO_PKG_AUTHORS"))
        .about(env!("CARGO_PKG_DESCRIPTION"))
        .after_help(
            "Get a GitHub access token here (no scopes needed):
    https://github.com/settings/tokens

Get a GitLab access token here (scope api):
    https://gitlab.com/profile/personal_access_tokens

",
        )
        .arg(
            Arg::with_name("KEYWORDS_YAML")
                .help("Sets the path to the keywords yaml file")
                .required(true),
        )
        .arg(
            Arg::with_name("github-token")
                .help("Sets the GitHub API Token")
                .long("github")
                .value_name("TOKEN")
                .required(true),
        )
        .arg(
            Arg::with_name("gitlab-token")
                .help("Sets the GitLab API Token")
                .long("gitlab")
                .value_name("TOKEN")
                .required(true),
        )
        .get_matches();

    let keywords_yaml_path = matches
        .value_of("KEYWORDS_YAML")
        .context("KEYWORDS_YAML not provided")?
        .to_string();
    let mut keywords: Map<String, Vec<Commit>> =
        serde_yaml::from_reader(File::open(&keywords_yaml_path)?)?;
    let commits = keywords.values().flat_map(|cs| cs);
    let authors = commits
        .flat_map(|c| c.rating.keys().map(|k| k.clone()))
        .collect::<HashSet<_>>();
    let github_token = matches
        .value_of("github-token")
        .context("github-token not defined")?;
    let gitlab_token = matches
        .value_of("gitlab-token")
        .context("gitlab-token not defined")?;

    let url_re = Regex::new(r"^https://(.+?)/(.+?)(:?\.git)?$")?;

    let (cb_sink_tx, cb_sink_rx) = channel();
    let (readonly_name_tx, readonly_name_rx) = channel();
    let siv_task_handle = task::spawn(async move {
        let mut siv = Cursive::default();
        cb_sink_tx.send(siv.cb_sink().clone()).unwrap();

        let mut tabs = TabPanel::new();

        let mut new_tab = LinearLayout::vertical();
        let readonly_name_new_tx = readonly_name_tx.clone();
        let readonly_name_ok_tx = readonly_name_tx.clone();
        let readonly_name_view_tx = readonly_name_tx.clone();
        let readonly_name_edit_tx = readonly_name_tx.clone();
        new_tab.add_child(TextView::new("Please enter your name"));
        new_tab.add_child(
            EditView::new()
                .on_submit(move |siv, name| {
                    if !name.is_empty() {
                        readonly_name_new_tx
                            .send((false, name.to_string()))
                            .unwrap();
                        siv.pop_layer();
                    }
                })
                .with_name("name_text_field"),
        );
        new_tab.add_child(Button::new("Ok", move |siv| {
            let name = siv
                .find_name::<EditView>("name_text_field")
                .unwrap()
                .get_content()
                .to_string();

            if !name.is_empty() {
                readonly_name_ok_tx.send((false, name)).unwrap();
                siv.pop_layer();
            }
        }));
        tabs.add_tab("New", new_tab);

        let mut view_tab = LinearLayout::vertical();
        view_tab.add_child(TextView::new(
            "Please select a rating (press enter) to view",
        ));
        let mut view_select = SelectView::new().on_submit(move |siv, author: &String| {
            readonly_name_view_tx.send((true, author.clone())).unwrap();
            siv.pop_layer();
        });
        for rating in &authors {
            view_select.add_item(rating.clone(), rating.clone());
        }
        view_tab.add_child(view_select);
        tabs.add_tab("View", view_tab);

        let mut edit_tab = LinearLayout::vertical();
        edit_tab.add_child(TextView::new(
            "Please select a rating (press enter) to edit",
        ));
        let mut edit_select = SelectView::new().on_submit(move |siv, author: &String| {
            readonly_name_edit_tx.send((false, author.clone())).unwrap();
            siv.pop_layer();
        });
        for rating in &authors {
            edit_select.add_item(rating.clone(), rating.clone());
        }
        edit_tab.add_child(edit_select);
        tabs.add_tab("Edit", edit_tab);

        siv.add_layer(tabs.max_width(60));
        siv.run();
    });
    let cb_sink = cb_sink_rx.recv().unwrap();
    let (readonly, name) = readonly_name_rx.recv().unwrap();

    let (paging_tx, paging_rx) = channel();
    let (quit_tx, quit_rx) = channel();
    cb_sink
        .send(Box::new(move |siv| {
            let mut keywords_layout = LinearLayout::vertical();
            keywords_layout.add_child(DummyView.full_screen());

            let mut buttons_layout = LinearLayout::horizontal();
            let prev_tx = paging_tx.clone();
            buttons_layout.add_child(
                Button::new("Prev", move |siv| {
                    siv.find_name::<Button>("prev").unwrap().disable();

                    let comment = siv
                        .find_name::<TextArea>("comment")
                        .unwrap()
                        .get_content()
                        .to_string();
                    let is_refactoring = siv
                        .find_name::<RadioButton<bool>>("is_refactoring")
                        .unwrap()
                        .is_selected();
                    prev_tx.send(Paging::Prev(comment, is_refactoring)).unwrap();
                })
                .disabled()
                .with_name("prev"),
            );
            let next_tx = paging_tx.clone();
            buttons_layout.add_child(
                Button::new("Next", move |siv| {
                    siv.find_name::<Button>("next").unwrap().disable();

                    let comment = siv
                        .find_name::<TextArea>("comment")
                        .unwrap()
                        .get_content()
                        .to_string();
                    let is_refactoring = siv
                        .find_name::<RadioButton<bool>>("is_refactoring")
                        .unwrap()
                        .is_selected();
                    next_tx.send(Paging::Next(comment, is_refactoring)).unwrap();
                })
                .disabled()
                .with_name("next"),
            );
            let finish_tx = paging_tx.clone();
            buttons_layout.add_child(
                Button::new("Finish", move |siv| {
                    siv.find_name::<Button>("finish").unwrap().disable();

                    let comment = siv
                        .find_name::<TextArea>("comment")
                        .unwrap()
                        .get_content()
                        .to_string();
                    let is_refactoring = siv
                        .find_name::<RadioButton<bool>>("is_refactoring")
                        .unwrap()
                        .is_selected();
                    finish_tx
                        .send(Paging::Finish(comment, is_refactoring))
                        .unwrap();
                })
                .disabled()
                .with_name("finish"),
            );
            keywords_layout.add_child(buttons_layout.align_bottom_right());
            let keywords_dialog = Panel::new(keywords_layout)
                .with_name("keywords_dialog")
                .full_screen();

            siv.add_layer(keywords_dialog);

            siv.add_global_callback('q', move |siv| {
                let quit_tx_cp = quit_tx.clone();
                siv.add_layer(
                    Dialog::text("Do you really want to quit and discard all changes?")
                        .button("Yes", move |_siv| quit_tx_cp.send(()).unwrap())
                        .button("No", move |siv| {
                            siv.pop_layer();
                        }),
                );
            });
        }))
        .unwrap();

    let mut save = true;
    let mut finished = false;
    let keys = keywords.keys().map(|k| k.clone()).collect::<Vec<_>>();
    let key_len = keys.len();
    let mut key_idx = 0;
    let mut commit_idx = 0;
    'outer: loop {
        let kw = &keys[key_idx];
        let commits = keywords.get(kw).unwrap();
        let commits_len = commits.len();
        let commit = &commits[commit_idx];

        let captures = url_re
            .captures(&commit.origin)
            .context("could not parse origin")?;
        let domain = captures
            .get(1)
            .context("no valid domain for origin")?
            .as_str();
        let path = captures
            .get(2)
            .context("no valid path for origin")?
            .as_str();
        let urlenc = utf8_percent_encode(path, NON_ALPHANUMERIC);

        let (msg_url, diff_url, auth) = match domain {
            "github.com" => (
                format!(
                    "https://api.github.com/repos/{}/git/commits/{}",
                    path, commit.commit
                ),
                format!("https://github.com/{}/commit/{}.diff", path, commit.commit),
                ("Authorization", format!("token {}", github_token)),
            ),
            "gitlab.com" => (
                format!(
                    "https://gitlab.com/api/v4/projects/{}/repository/commits/{}",
                    urlenc, commit.commit
                ),
                format!(
                    "https://gitlab.com/{}/-/commit/{}.diff",
                    path, commit.commit
                ),
                ("PRIVATE-TOKEN", gitlab_token.to_string()),
            ),
            d => bail!("invalid domain {}", d),
        };

        let message_request = surf::get(msg_url)
            .set_header(auth.0, auth.1.clone())
            .recv_json::<Value>();
        let diff_request = surf::get(diff_url).set_header(auth.0, auth.1).recv_string();

        let (tx, rx) = channel();
        async_std::task::spawn(async move {
            let result = message_request.join(diff_request).await;
            tx.send(result).expect("sending over channel failed");
        });

        let keyword = kw.clone();
        let commit_clone = commit.clone();
        let name_clone = name.clone();
        let inner_cb_sink = cb_sink.clone();

        cb_sink
            .send(Box::new(move |mut siv| {
                let keyword = keyword.clone();
                let commit_clone = commit_clone.clone();

                let mut keywords_dialog = siv
                    .find_name::<Panel<LinearLayout>>("keywords_dialog")
                    .unwrap();
                keywords_dialog.set_title(format!(
                    "Loading '{keyword}' / {section} | {origin} @ {commit} - {date}",
                    keyword = keyword,
                    section = "N/A",
                    origin = commit_clone.origin,
                    date = "N/A",
                    commit = commit_clone.commit,
                ));

                let async_view = AsyncView::new(&mut siv, move || {
                    let (message_result, diff_result) = match rx.try_recv() {
                        Ok(req) => req,
                        Err(_) => return AsyncState::Pending,
                    };

                    let mut linear = LinearLayout::vertical();

                    let message = match message_result {
                        Ok(message) => message["message"]
                            .as_str()
                            .unwrap_or("!! Commit message not available !!")
                            .to_string(),
                        Err(err) => format!("{}", err),
                    };

                    linear.add_child(
                        Panel::new(
                            TextView::new(message)
                                .scrollable()
                                .scroll_x(false)
                                .scroll_y(true),
                        )
                        .title("Commit Message")
                        .title_position(HAlign::Left),
                    );

                    let diff = match diff_result {
                        Ok(diff) => diff,
                        Err(err) => format!("{}", err),
                    };

                    linear.add_child(
                        Panel::new(
                            TextView::new(diff)
                                .scrollable()
                                .scroll_x(false)
                                .scroll_y(true),
                        )
                        .title("Diff")
                        .title_position(HAlign::Left)
                        .full_screen(),
                    );

                    let mut rating_layout = LinearLayout::vertical();
                    let mut radio_group = RadioGroup::new();

                    let mut valid_btn =
                        radio_group.button(true, "This commit is a valid refactoring");
                    let mut invalid_btn =
                        radio_group.button(false, "This commit does not contain refactoring");

                    let mut comment_area = TextArea::new().content(
                        match commit_clone.rating.get_key_value(&name_clone) {
                            Some(val) => val.1.comment.clone(),
                            None => "".to_string(),
                        },
                    );

                    match commit_clone.rating.get_key_value(&name_clone) {
                        Some(val) => {
                            if val.1.is_refactoring {
                                valid_btn.select();
                            } else {
                                invalid_btn.select();
                            }
                        }
                        None => {
                            invalid_btn.select();
                        }
                    }

                    if readonly {
                        valid_btn.disable();
                        invalid_btn.disable();
                        comment_area.disable();
                    }

                    rating_layout.add_child(valid_btn.with_name("is_refactoring"));
                    rating_layout.add_child(invalid_btn);
                    rating_layout.add_child(TextView::new("\nComment:"));
                    rating_layout.add_child(comment_area.with_name("comment").min_height(3));

                    linear.add_child(
                        Panel::new(rating_layout)
                            .title("Refactor rating")
                            .title_position(HAlign::Left),
                    );

                    let keyword = keyword.clone();
                    let commit_clone = commit_clone.clone();
                    inner_cb_sink
                        .send(Box::new(move |siv| {
                            siv.find_name::<Button>("prev").unwrap().enable();
                            siv.find_name::<Button>("next").unwrap().enable();
                            siv.find_name::<Button>("finish").unwrap().disable();
                            if commit_idx == 0 && key_idx == 0 {
                                siv.find_name::<Button>("prev").unwrap().disable();
                            }
                            if key_idx + 1 >= key_len && commit_idx + 1 >= commits_len {
                                siv.find_name::<Button>("next").unwrap().disable();
                                siv.find_name::<Button>("finish").unwrap().enable();
                            }
                            let mut keywords_dialog = siv
                                .find_name::<Panel<LinearLayout>>("keywords_dialog")
                                .unwrap();
                            keywords_dialog.set_title(format!(
                                "'{keyword}' / {section} | {origin} @ {commit} - {date}",
                                keyword = keyword,
                                origin = commit_clone.origin,
                                section = "N/A",
                                commit = commit_clone.commit,
                                date = "N/A",
                            ));
                        }))
                        .unwrap();

                    AsyncState::Available(linear)
                });

                let keywords_layout = keywords_dialog.get_inner_mut();
                keywords_layout.remove_child(0);
                keywords_layout.insert_child(0, async_view.full_screen());
            }))
            .unwrap();

        let comment;
        let is_refactoring;

        let old_commit_idx = commit_idx;
        loop {
            match quit_rx.try_recv() {
                Ok(_) => {
                    save = false;
                    break 'outer;
                }
                Err(_) => {}
            }

            match paging_rx.try_recv() {
                Ok(Paging::Next(c, r)) => {
                    comment = c;
                    is_refactoring = r;
                    if commit_idx + 1 >= commits_len {
                        key_idx += 1;
                        commit_idx = 0;
                    } else {
                        commit_idx += 1;
                    }
                    break;
                }
                Ok(Paging::Prev(c, r)) => {
                    comment = c;
                    is_refactoring = r;
                    if commit_idx == 0 {
                        key_idx -= 1;
                        let kw = &keys[key_idx];
                        let commits = keywords.get(kw).unwrap();
                        commit_idx = commits.len() - 1;
                    } else {
                        commit_idx -= 1;
                    }
                    break;
                }
                Ok(Paging::Finish(c, r)) => {
                    comment = c;
                    is_refactoring = r;
                    finished = true;
                    break;
                }
                Err(_) => thread::sleep(Duration::from_millis(10)),
            }
        }

        keywords.get_mut(kw).unwrap()[old_commit_idx].rating.insert(
            name.clone(),
            Rating {
                is_refactoring: is_refactoring,
                comment: comment,
            },
        );

        if finished {
            break 'outer;
        }
    }

    if save {
        serde_yaml::to_writer(File::create(&keywords_yaml_path)?, &keywords)?;
    }

    cb_sink
        .send(Box::new(move |siv| {
            siv.pop_layer();

            if save {
                siv.add_layer(
                    Dialog::text(format!(
                        "Rating successfully saved to {}",
                        keywords_yaml_path
                    ))
                    .button("Ok", |siv| siv.quit()),
                );
            } else {
                siv.quit();
            }
        }))
        .unwrap();
    siv_task_handle.await;

    Ok(())
}

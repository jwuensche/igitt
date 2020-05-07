<h1 align="center">Welcome to igitt 🔍</h1>
<p align="center">
  <a href="https://github.com/jwuensche/igitt/blob/master/LICENSE">
    <img alt="GitHub" src="https://img.shields.io/github/license/jwuensche/igitt.svg">
  </a>
  <a href="http://spacemacs.org">
    <img src="https://cdn.rawgit.com/syl20bnr/spacemacs/442d025779da2f62fc86c2082703697714db6514/assets/spacemacs-badge.svg" />
  </a>
  <a href="http://makeapullrequest.com">
    <img alt="PRs Welcome" src="https://img.shields.io/badge/PRs-welcome-brightgreen.svg">
  </a>
  <br>
  <i>A curses-like terminal application to validate a sample of git commits</i>
</p>

---

## What is this tool about?
This tool should ease your work to check for the validity and soundness of an amount of git commits.
We originally developed this tool for a repository mining reasearch paper, in which we classified commits due to certain occuring phrases and words in their messages. Checking all of them by hand is a lot of work but necessary to prove your approach is correct, so we developed a tool for it `igitt`.

## Preparation
To start you need a gitlab and github API key, you can generate them in your respecting profiles (more on that on [github](https://help.github.com/en/github/authenticating-to-github/creating-a-personal-access-token-for-the-command-line) or [gitlab](https://docs.gitlab.com/ee/user/profile/personal_access_tokens.html)).  
> Be sure to check the `api` field in the gitlab token creation.

Once you've done that be sure to save them as they cannot be reviewed again.

For Linux-based and MacOS there are pre-built binary available to download in the github releases. If you're not on one of these platforms, have a look at the `Building` section of the README.

```bash
# Linux statically linked binary
$ curl -L https://github.com/jwuensche/igitt/releases/download/v0.1.2/igitt-linux-amd64 --output igitt && chmod +x igitt

# MacOS binary
$ curl -L https://github.com/jwuensche/igitt/releases/download/v0.1.2/igitt-macos --output igitt && chmod +x igitt
```

## Usage

To start evaluating the example data
```bash
$ curl https://raw.githubusercontent.com/jwuensche/igitt/master/data/keyword_check.yaml --output example_data.yml
$ ./igitt --gitlab your-gitlab-token --github your-github-token example_data.yml
```

Then you will be prompted by a dialog asking you too either create a new evaluation, view an existing one or edit one.
Select new and enter your name and then you can start evaluating.

The program is quitable anytime with `q`, it will ask you to save your progress. The next time you can just continue by selecting your name in `Edit` at the beginning, it will ask you to continue from your last entry.

## Keybinds

There are a few keybinds for you to use to make evaluating faster:

| Key | Action                 |
|-----|------------------------|
| `q` | Quit                   |
| `y` | This is a refactoring  |
| `n` | This is no refactoring |
| `i` | This commit is invalid |
| `,` | Go to prev commit      |
| `.` | Go to next commit      |

## Evaluation
Once all result have been entered by the user you can start the evaluation. This can be done by just opening the tool again or via the -e flag.
```bash
$ ./igitt --gitlab your-gitlab-token --github your-github-token example_data.yml
# or
$ ./igitt --gitlab your-gitlab-token --github your-github-token example_data.yml
```

## How to get working with your own commits?
Until now we have used the example commits, from this repository.
But you probably want to use your own sample of commits, for that you have to create your own YAML file. 
We created a schema for this to be read in by this tool, the structure of it we will explain here.

The bare minimum information:
```yaml
keyword:
  - origin: _url_of_a_repository_on_github_or_gitlab_
    commit: _the_full_commit_hash_you_want_to_inspect_
```

But it can be extended to contain a few more additional information.
```yaml
keyword:
  - origin: _url_of_a_repository_on_github_or_gitlab_
    commit: _the_full_commit_hash_you_want_to_inspect_
    section: (Under which subsection does this commit fall into e.g. estimated to be highly probable to be a true positive)
    time: 1970-01 (arbitrary time string for your own choice)
```

## Building

To build the project for development purposes be sure to have the rust tooling installed ([rustup](https://rustup.rs/)).
> We use the `termion` backend for `cursive` it **should** work on almost all systems, if you have anyway troubles using it on your system check out other available backends available for cursive.
>
> Our tests have been performed on GNU/Linux based systems and MacOS Catalina.

```bash
$ cargo build
```

If you want to actually use the tool for data validation, it is advised that you build for the release target.
```bash
$ cargo build --release
```

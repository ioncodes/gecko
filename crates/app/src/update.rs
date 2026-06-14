use serde::Deserialize;

const REPO: &str = "ioncodes/gecko";
const SELF_SHA: &str = env!("GECKO_GIT_SHA");
const SELF_DATE: &str = env!("GECKO_COMMIT_DATE");

const RELEASE_PAGE: &str = "https://github.com/ioncodes/gecko/releases/tag/release";
const NIGHTLY_PAGE: &str = "https://github.com/ioncodes/gecko/releases/tag/nightly";

#[derive(Debug, Clone)]
pub enum Outcome {
    UpToDate,
    Available(String),
    Unpublished,
    Failed(String),
}

enum CompareError {
    NotPublished,
    Failed(String),
}

#[derive(Deserialize)]
struct Comparison {
    ahead_by: u32,
    behind_by: u32,
}

#[derive(Deserialize)]
struct Commit {
    commit: CommitBody,
}

#[derive(Deserialize)]
struct CommitBody {
    committer: CommitActor,
}

#[derive(Deserialize)]
struct CommitActor {
    date: String,
}

pub async fn check() -> Outcome {
    if SELF_SHA.is_empty() {
        return Outcome::Failed("this build has no embedded commit".to_owned());
    }

    let client = match reqwest::Client::builder().user_agent("gecko-updater").build() {
        Ok(client) => client,
        Err(err) => return Outcome::Failed(err.to_string()),
    };

    let release = match self::compare(&client, "release").await {
        Ok(cmp) => cmp,
        Err(CompareError::NotPublished) => return Outcome::Unpublished,
        Err(CompareError::Failed(err)) => return Outcome::Failed(err),
    };

    if release.behind_by == 0 {
        return if release.ahead_by > 0 {
            Outcome::Available(RELEASE_PAGE.to_owned())
        } else {
            Outcome::UpToDate
        };
    }

    let nightly_fresh = matches!(self::compare(&client, "nightly").await, Ok(cmp) if cmp.ahead_by > 0);
    if nightly_fresh {
        return Outcome::Available(NIGHTLY_PAGE.to_owned());
    }

    if self::release_is_newer(&client).await {
        return Outcome::Available(RELEASE_PAGE.to_owned());
    }

    Outcome::UpToDate
}

async fn compare(client: &reqwest::Client, head: &str) -> Result<Comparison, CompareError> {
    let url = format!("https://api.github.com/repos/{REPO}/compare/{SELF_SHA}...{head}?per_page=1");
    let resp = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .map_err(|err| CompareError::Failed(err.to_string()))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Err(CompareError::NotPublished);
    }

    if !resp.status().is_success() {
        return Err(CompareError::Failed(format!("GitHub API returned {}", resp.status())));
    }

    resp.json::<Comparison>()
        .await
        .map_err(|err| CompareError::Failed(err.to_string()))
}

async fn release_is_newer(client: &reqwest::Client) -> bool {
    if SELF_DATE.is_empty() {
        return false;
    }

    let url = format!("https://api.github.com/repos/{REPO}/commits/release");
    let fetched = client
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .send()
        .await
        .ok()
        .filter(|resp| resp.status().is_success());

    let Some(resp) = fetched else {
        return false;
    };

    let Ok(commit) = resp.json::<Commit>().await else {
        return false;
    };

    commit.commit.committer.date.as_str() > SELF_DATE
}

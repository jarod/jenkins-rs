use std::{collections::HashMap, time::Duration};

use anyhow::{bail, Context, Result};
use log::{error, info, trace, warn};
use reqwest::RequestBuilder;
use serde::Deserialize;
use tokio::time::sleep;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("API error: {0}")]
    APIError(String),
    #[error("Queue item not exists, maybe already running or finished")]
    QueueItemNotExists,
    #[error("Network error: {0}")]
    NetworkError(reqwest::Error),
}

/// [Jenkins : Remote access API](https://wiki.jenkins.io/display/JENKINS/Remote+access+API)
///
pub struct Jenkins {
    hc: reqwest::Client,
    url: String,
    user: String,
    password: String,
}

impl Jenkins {
    /// Create Jenkins instance
    ///
    /// ## Arguments
    ///
    /// * `password` - password or api token of user
    ///
    pub fn new(url: &str, user: &str, password: &str) -> Jenkins {
        let hc = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .build()
            .expect("failed to init http client");
        Jenkins {
            hc,
            url: url.to_owned(),
            user: user.to_owned(),
            password: password.to_owned(),
        }
    }

    pub fn get_url(&self) -> &str {
        &self.url
    }

    fn post(&self, url: &str) -> RequestBuilder {
        self.hc
            .post(url)
            .basic_auth(&self.user, Some(&self.password))
    }

    fn get(&self, url: &str) -> RequestBuilder {
        self.hc
            .get(url)
            .basic_auth(&self.user, Some(&self.password))
    }

    /// Poll from new build queue item url until build number available
    ///
    /// [reference](https://docs.cloudbees.com/docs/cloudbees-ci-kb/latest/client-and-managed-controllers/get-build-number-with-rest-api)
    ///
    /// ## Arguments
    ///
    /// * `queue_item_url` - `location` field in `build`/`buildWithParameters` response header
    ///
    pub async fn poll_queue_item(
        &self,
        queue_item_url: &str,
    ) -> Result<QueueItemRes, anyhow::Error> {
        let queue_url = format!("{}api/json", queue_item_url);
        loop {
            sleep(Duration::from_secs(3)).await;
            match self.get(&queue_url).send().await {
                Ok(queue_res) => {
                    info!("queue_res={:?}", queue_res);
                    if queue_res.status().is_client_error() {
                        bail!(Error::QueueItemNotExists)
                    }
                    let qi_res: QueueItemRes = queue_res
                        .json()
                        .await
                        .context("parse queue item payload as json")?;
                    if qi_res.executable.is_some() {
                        info!("Get {}: body={:?}", queue_url, qi_res);
                        return Ok(qi_res);
                    } else {
                        trace!("Get {}: body={:?}", queue_url, qi_res);
                    }
                }
                Err(err) => {
                    error!("Get {}: err={:?}", queue_url, err);
                    // Err(err)
                    bail!(Error::NetworkError(err));
                }
            }
        }
    }

    /// [Parameterized Build](https://wiki.jenkins.io/display/JENKINS/Parameterized-Build.html)
    ///
    /// ## Arguments
    ///
    /// * `job` - job name
    /// * `params` - parameters to trigger a build
    ///
    pub async fn build_with_parameter(
        &self,
        job: &str,
        params: HashMap<&str, &str>,
    ) -> Result<QueueItemRes> {
        let url = format!("{}/job/{}/buildWithParameters", self.url, job);
        match self.post(&url).form(&params).send().await {
            Ok(res) => {
                if res.status().is_success() {
                    info!("buildWithParameters - job={}, res={:?}", job, res);
                    if let Some(location) = res.headers().get("location") {
                        let queue_url = location.to_str().expect("location header");
                        Ok(self.poll_queue_item(queue_url).await?)
                    } else {
                        bail!(Error::APIError("location header not available".to_owned()))
                    }
                } else {
                    warn!("buildWithParameters - job={}, res={:?}", job, res);
                    bail!(Error::APIError(format!("http status: {}", res.status())))
                }
            }
            Err(err) => {
                error!("buildWithParameters - job={}, err={:?}", job, err);
                bail!(err)
            }
        }
    }
}

#[derive(Deserialize, Debug)]
pub struct QueueItemExecutable {
    pub number: i32,
    pub url: String,
}
#[derive(Deserialize, Debug)]
pub struct QueueItemRes {
    pub why: Option<String>,
    pub executable: Option<QueueItemExecutable>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    #[tokio::test]
    async fn build_with_parameter() {
        let _ = env_logger::builder().is_test(true).try_init();

        let cli = Jenkins::new(
            "https://jenkins.domain.com",
            "jenkins-user",
            "jenkins-token",
        );
        let params = HashMap::from([("HostLimit", "xxx"), ("Module", "ansible.builtin.ping")]);
        let res = cli
            .build_with_parameter("ansible-global-adhoc", params)
            .await;
        println!("{:?}", res);
    }
}

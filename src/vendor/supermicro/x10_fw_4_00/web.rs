use anyhow::{Context, Result, bail, ensure};
use regex::Regex;
use reqwest::{Url, blocking::Client};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

pub struct WebSession {
    client: Client,
    pub origin: Url,
    pub ws_url: String,
    pub cookie: String,
    pub ticket: String,
    pub insecure: bool,
    logged_out: AtomicBool,
}

fn session_cookie(headers: &reqwest::header::HeaderMap) -> Result<String> {
    // This firmware returns both a clearing SID with no Path and a live SID
    // with Path=/. Select one explicitly, avoiding duplicate SID cookies at /cgi.
    headers
        .get_all(reqwest::header::SET_COOKIE)
        .iter()
        .filter_map(|h| h.to_str().ok())
        .filter_map(|h| {
            h.strip_prefix("SID=")
                .map(|v| v.split(';').next().unwrap_or(""))
        })
        .rfind(|v| !v.is_empty())
        .map(|v| format!("SID={v}"))
        .context("AUTH_FAILED: no session Cookie returned")
}

pub fn entry_ticket(html: &str) -> Result<String> {
    let tags = Regex::new(r"(?is)<input\b[^>]*>")?;
    let attrs = Regex::new(r#"([\w-]+)\s*=\s*["']([^"']*)["']"#)?;
    for tag in tags.find_iter(html) {
        let map = attrs
            .captures_iter(tag.as_str())
            .map(|c| (c[1].to_lowercase(), c[2].to_string()))
            .collect::<std::collections::HashMap<_, _>>();
        if map.get("id").is_some_and(|v| v == "entry_value") {
            let ticket = map.get("value").context("KVM token missing value")?;
            ensure!(
                (24..=256).contains(&ticket.len()) && ticket.is_ascii(),
                "UNSUPPORTED_PROFILE: unexpected KVM ticket format"
            );
            return Ok(ticket.clone());
        }
    }
    bail!("AUTH_FAILED: launcher did not return a KVM ticket (web session expired or unavailable)")
}

impl WebSession {
    pub fn login(target: &str, user: &str, password: &str, insecure: bool) -> Result<Self> {
        let raw = if target.contains("://") {
            target.to_string()
        } else {
            format!("https://{target}")
        };
        let mut origin = Url::parse(&raw).context("INVALID_ARGUMENT: invalid BMC URL")?;
        ensure!(
            matches!(origin.scheme(), "https" | "http") && origin.host_str().is_some(),
            "INVALID_ARGUMENT: expected HTTP(S) BMC target"
        );
        ensure!(
            origin.username().is_empty()
                && origin.password().is_none()
                && origin.path() == "/"
                && origin.query().is_none()
                && origin.fragment().is_none(),
            "INVALID_ARGUMENT: target must be an origin, without credentials or path"
        );
        ensure!(
            origin.scheme() == "https" || insecure,
            "INVALID_ARGUMENT: HTTP requires explicit insecure mode"
        );
        origin.set_path("/");
        let client = Client::builder()
            .no_proxy()
            .http1_only()
            .http1_title_case_headers()
            .danger_accept_invalid_certs(insecure)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(20))
            .user_agent("ikvmt/0.2 native")
            .build()?;
        client
            .get(origin.clone())
            .send()?
            .error_for_status()?
            .text()?;
        let login_response = client
            .post(origin.join("cgi/login.cgi")?)
            .header("Referer", origin.as_str())
            .form(&[("name", user), ("pwd", password)])
            .send()
            .context("BMC HTTP login request failed")?
            .error_for_status()?;
        let cookie = session_cookie(login_response.headers())?;
        let mut ws = origin.clone();
        ws.set_scheme(if origin.scheme() == "https" {
            "wss"
        } else {
            "ws"
        })
        .map_err(|_| anyhow::anyhow!("invalid WS scheme"))?;
        // From here on, all early errors release the authenticated web session.
        let mut session = Self {
            client: client.clone(),
            origin: origin.clone(),
            ws_url: ws.to_string(),
            cookie: cookie.clone(),
            ticket: String::new(),
            insecure,
            logged_out: AtomicBool::new(false),
        };
        let login = login_response.text()?;
        ensure!(
            login.contains("mainmenu"),
            "AUTH_FAILED: BMC rejected login"
        );
        let main = origin.join("cgi/url_redirect.cgi?url_name=mainmenu")?;
        client
            .get(main.clone())
            .header("Cookie", &cookie)
            .header("Referer", origin.as_str())
            .send()?
            .error_for_status()?
            .text()?;
        let launcher = origin.join("cgi/url_redirect.cgi?url_name=man_ikvm_html5")?;
        let html = client
            .get(launcher.clone())
            .header("Cookie", &cookie)
            .header("Referer", main.as_str())
            .send()?
            .error_for_status()?
            .text()?;
        ensure!(
            !html.contains("logout_alert"),
            "AUTH_FAILED: BMC session was not established"
        );
        let csrf = Regex::new(r#"SmcCsrfInsert\s*\(\s*["']CSRF_TOKEN["']\s*,\s*["']([^"']+)["']"#)?
            .captures(&html)
            .map(|c| c[1].to_string());
        // Mirror the firmware's read-only readiness queries, including its CSRF header.
        let mut ports = client
            .post(origin.join("cgi/ipmi.cgi")?)
            .header("Cookie", &cookie)
            .header("Referer", launcher.as_str())
            .form(&[("GETPORTSINFO.XML", "(0,0)")]);
        if let Some(ref c) = csrf {
            ports = ports.header("CSRF_TOKEN", c);
        }
        let status = ports.send()?.error_for_status()?.text()?;
        ensure!(
            !status.contains("IKVM_SERVICE=\"0\""),
            "BMC_BUSY: iKVM service is disabled"
        );
        let mut ready = client
            .post(origin.join("cgi/upgrade_process.cgi")?)
            .header("Cookie", &cookie)
            .header("Referer", launcher.as_str())
            .form(&[("fwtype", "255")]);
        if let Some(ref c) = csrf {
            ready = ready.header("CSRF_TOKEN", c);
        }
        ready.send()?.error_for_status()?.text()?;
        let boot = client
            .get(origin.join("cgi/url_redirect.cgi?url_name=man_ikvm_html5_bootstrap")?)
            .header("Cookie", &cookie)
            .header("Referer", launcher.as_str())
            .send()?
            .error_for_status()?
            .text()?;
        session.ticket = entry_ticket(&boot)?;
        Ok(session)
    }
    pub fn logout(&self) {
        if self.logged_out.swap(true, Ordering::Relaxed) {
            return;
        }
        if let Ok(url) = self.origin.join("cgi/logout.cgi") {
            let _ = self
                .client
                .get(url)
                .header("Cookie", &self.cookie)
                .header("Referer", self.origin.as_str())
                .timeout(Duration::from_secs(3))
                .send();
        }
    }
}

impl Drop for WebSession {
    fn drop(&mut self) {
        self.logout();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ticket_is_not_the_bmc_password() {
        assert_eq!(
            entry_ticket("<input value='0123456789abcdefghijklmn' id='entry_value'>").unwrap(),
            "0123456789abcdefghijklmn"
        );
        assert!(entry_ticket("<body onload='logout_alert()'>").is_err());
        assert!(entry_ticket("<input id='entry_value' value='short'>").is_err());
    }
    #[test]
    fn ignores_clearing_cookie_and_selects_only_live_sid() {
        use reqwest::header::{HeaderMap, SET_COOKIE};
        let mut h = HeaderMap::new();
        h.append(
            SET_COOKIE,
            "SID=; expires=Thursday,01-Jan-1970 00:00:00 GMT; ;Secure"
                .parse()
                .unwrap(),
        );
        assert!(session_cookie(&h).is_err());
        h.append(
            SET_COOKIE,
            "SID=test-token; path=/ ; ;Secure; HttpOnly"
                .parse()
                .unwrap(),
        );
        assert_eq!(session_cookie(&h).unwrap(), "SID=test-token");
    }
}

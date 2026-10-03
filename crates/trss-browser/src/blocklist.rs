//! The ad and tracker addresses a run's pages do not load.
//!
//! They cover the page and its frames with fewer requests and less memory
//! (ticket 0032: 103 requests became 87 and the process memory fell with
//! them). The patterns are the glob form `Network.setBlockedURLs` takes. The
//! authentication services (`challenges.cloudflare.com` for Cloudflare
//! Turnstile, Google's sign-in and Drive hosts) are never in the list.

pub const BLOCKED_URLS: &[&str] = &[
    "*://*.googlesyndication.com/*",
    "*://*.doubleclick.net/*",
    "*://*.googletagmanager.com/*",
    "*://*.google-analytics.com/*",
    "*://*.adtrafficquality.google/*",
    "*://*.googleadservices.com/*",
    "*://*.googletagservices.com/*",
    "*://*.2mdn.net/*",
    "*://adservice.google.*/*",
];

#[cfg(test)]
mod tests {
    use super::*;

    /// `*` matches any run of characters, as in the DevTools patterns.
    fn matches(pattern: &str, url: &str) -> bool {
        let parts: Vec<String> = pattern.split('*').map(regex::escape).collect();
        regex::Regex::new(&format!("^{}$", parts.join(".*")))
            .unwrap()
            .is_match(url)
    }

    fn blocked(url: &str) -> bool {
        BLOCKED_URLS.iter().any(|p| matches(p, url))
    }

    #[test]
    fn ads_and_trackers_are_blocked() {
        for url in [
            "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js",
            "https://tpc.googlesyndication.com/sodar/x.js",
            "https://googleads.g.doubleclick.net/pagead/ads?x=1",
            "https://www.googletagmanager.com/gtag/js?id=G-1",
            "https://region1.google-analytics.com/g/collect",
            "https://ep1.adtrafficquality.google/getconfig/sodar",
            "https://adservice.google.co.kr/adsid/integrator.js",
        ] {
            assert!(blocked(url), "{url}");
        }
    }

    #[test]
    fn the_authentication_services_and_the_files_are_not() {
        for url in [
            "https://challenges.cloudflare.com/turnstile/v0/api.js",
            "https://challenges.cloudflare.com/cdn-cgi/challenge-platform/h/b/x",
            "https://erulabo.com/859",
            "https://accounts.google.com/signin",
            "https://drive.usercontent.google.com/download?id=1&export=download",
            "https://drive.google.com/file/d/1/view",
            "https://www.google.com/recaptcha/api.js",
        ] {
            assert!(!blocked(url), "{url}");
        }
    }
}

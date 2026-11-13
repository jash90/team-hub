//! A fake Jira with a realistic project of 60 issues, for trying the import by hand:
//!
//! ```text
//! cargo run -p hub-mod-jira --example fake_jira -- 8195 cloud
//! cargo run -p hub-mod-jira --example fake_jira -- 8196 server
//! ```
//!
//! Cloud accepts the e-mail `admin@example.com` with the token `demo-token-1234`; Server
//! accepts `demo-token-1234` as a personal access token (no e-mail).

#[path = "../tests/common/fake.rs"]
mod fake;

use base64::Engine;

const EMAIL: &str = "admin@example.com";
const TOKEN: &str = "demo-token-1234";

#[tokio::main]
async fn main() {
    let mut arguments = std::env::args().skip(1);
    let port: u16 = arguments.next().and_then(|port| port.parse().ok()).unwrap_or(8195);
    let cloud = arguments.next().is_none_or(|flavour| flavour != "server");
    let accepts = if cloud {
        format!("Basic {}", base64::engine::general_purpose::STANDARD.encode(format!("{EMAIL}:{TOKEN}")))
    } else {
        format!("Bearer {TOKEN}")
    };
    let mut upstream = fake::demo(cloud, &accepts);

    // Small pages, so an import of 60 issues takes a few of them.
    upstream.page_size = 20;
    upstream.delay = arguments.next().and_then(|delay| delay.parse().ok()).unwrap_or(0);

    let issues = upstream.issues.len();
    let fake = fake::Fake::serve(upstream, &format!("127.0.0.1:{port}")).await;

    println!("A fake Jira {} with {issues} issues of the project WEB is at {}", if cloud { "Cloud" } else { "Server" }, fake.base_url);
    println!("{}token: {TOKEN}", if cloud { format!("e-mail: {EMAIL}, ") } else { String::new() });

    let _ = tokio::signal::ctrl_c().await;
}

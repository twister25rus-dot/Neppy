//! Describing a launch, without sending it anywhere.
//!
//! Examples are compiled and linted in CI, so they cannot drift from the API.
//! This one builds the plan and stops short of the network, so it runs on a
//! machine with no hosting credentials:
//!
//! ```sh
//! cargo run --example basic
//! ```
//!
//! To actually ship it, set `TINYHOSTS_VERCEL_TOKEN` and add:
//!
//! ```no_run
//! # use tinyhosts::{LaunchPlan, ProviderKind};
//! # async fn ship(plan: LaunchPlan) -> tinyhosts::Result<()> {
//! let host = tinyhosts::connect_from_env(ProviderKind::Vercel)?;
//! let launched = tinyhosts::launch(host.as_ref(), &plan).await?;
//! println!("building at {:?}", launched.url());
//! # Ok(())
//! # }
//! ```

use tinyhosts::{Bundle, DatabaseSpec, EnvVar, LaunchPlan, ProviderKind, Result, SiteSpec};

fn main() -> Result<()> {
    let mut bundle = Bundle::new();
    bundle.insert("package.json", br#"{"name":"shop","private":true}"#)?;
    bundle.insert(
        "app/page.tsx",
        b"export default function Page() { return <h1>Shop</h1>; }",
    )?;

    let plan = LaunchPlan::new(SiteSpec::new("shop"), bundle)
        .with_database(DatabaseSpec::new("shop-db"))
        .with_env(vec![EnvVar::new("NEXT_PUBLIC_NAME", "Shop")])
        .with_domains(vec!["shop.example".to_owned()])
        .into_production();

    plan.validate()?;
    println!(
        "{} files, {} bytes, database {:?}, target {}",
        plan.bundle.len(),
        plan.bundle.total_bytes(),
        plan.database.as_ref().map(|database| &database.name),
        plan.target.as_str(),
    );

    // Failure modes are part of the public contract; show one.
    match LaunchPlan::new(SiteSpec::new("shop"), Bundle::new()).validate() {
        Ok(()) => println!("an empty bundle was accepted, which should not happen"),
        Err(error) => println!("expected failure: {error}"),
    }

    println!("providers in this build: {:?}", tinyhosts::rpc::providers());
    println!(
        "credentials come from {:?}",
        ProviderKind::Vercel.api_key_variables()
    );

    Ok(())
}

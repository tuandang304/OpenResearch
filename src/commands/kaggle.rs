//! `orx kaggle <args>`: the Kaggle CLI, unrestricted, signed in as a saved
//! account, so an agent reaches Kaggle with the credentials the user saved in
//! Settings → Compute → Kaggle without seeing them.

use crate::error::Result;

pub async fn run(args: crate::KaggleArgs) -> Result<()> {
    let code = crate::jobs::kaggle::passthrough(args.account.as_deref(), &args.args).await?;
    if code != 0 {
        std::process::exit(code);
    }
    Ok(())
}

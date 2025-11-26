//! TDF (Trusted Data Format) CLI commands for encryption, decryption, and P2P transport.

use anyhow::{Context, Result};
use clap::Subcommand;
use std::path::PathBuf;
use tokio::fs;

#[derive(Debug, Subcommand)]
pub enum TdfCommand {
    /// Encrypt a file using TDF format
    Encrypt {
        /// Input file to encrypt
        #[arg(short, long)]
        input: PathBuf,

        /// Output file path (default: <input>.tdf.json)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Attribute namespace (e.g., "https://arkavo.net/attr/classification")
        #[arg(short = 'n', long, default_value = "https://arkavo.net/attr/sensitivity")]
        namespace: String,

        /// Attribute values (can specify multiple)
        #[arg(short = 'v', long, default_values_t = vec!["internal".to_string()])]
        values: Vec<String>,

        /// KAS URL for key wrapping
        #[arg(long, default_value = "https://100.arkavo.net")]
        kas_url: String,
    },

    /// Decrypt a TDF file
    Decrypt {
        /// Input TDF file to decrypt
        #[arg(short, long)]
        input: PathBuf,

        /// Output file path (default: strips .tdf.json extension)
        #[arg(short, long)]
        output: Option<PathBuf>,

        /// OAuth client ID for KAS authentication
        #[arg(long, env = "ARKAVO_CLIENT_ID")]
        client_id: Option<String>,

        /// OAuth client secret
        #[arg(long, env = "ARKAVO_CLIENT_SECRET")]
        client_secret: Option<String>,
    },

    /// Stage encrypted data to Iroh P2P network (requires --features iroh)
    #[cfg(feature = "iroh")]
    Stage {
        /// Input TDF file to stage
        #[arg(short, long)]
        input: PathBuf,

        /// Print only the ticket (for piping)
        #[arg(short, long)]
        quiet: bool,
    },

    /// Fetch data from Iroh P2P network (requires --features iroh)
    #[cfg(feature = "iroh")]
    Fetch {
        /// Iroh blob ticket
        #[arg(short, long)]
        ticket: String,

        /// Output file path
        #[arg(short, long)]
        output: PathBuf,
    },

    /// Show info about a TDF file
    Info {
        /// TDF file to inspect
        #[arg(short, long)]
        input: PathBuf,
    },
}

pub async fn handle_tdf_command(command: TdfCommand) -> Result<()> {
    match command {
        TdfCommand::Encrypt {
            input,
            output,
            namespace,
            values,
            kas_url,
        } => handle_encrypt(input, output, namespace, values, kas_url).await,

        TdfCommand::Decrypt {
            input,
            output,
            client_id,
            client_secret,
        } => handle_decrypt(input, output, client_id, client_secret).await,

        #[cfg(feature = "iroh")]
        TdfCommand::Stage { input, quiet } => handle_stage(input, quiet).await,

        #[cfg(feature = "iroh")]
        TdfCommand::Fetch { ticket, output } => handle_fetch(ticket, output).await,

        TdfCommand::Info { input } => handle_info(input).await,
    }
}

async fn handle_encrypt(
    input: PathBuf,
    output: Option<PathBuf>,
    namespace: String,
    values: Vec<String>,
    kas_url: String,
) -> Result<()> {
    use arkavo_tdf::{OpenTdfService, PolicyBuilder, TdfEncryptor};

    println!("Encrypting {}...", input.display());

    let plaintext = fs::read(&input)
        .await
        .with_context(|| format!("Failed to read {}", input.display()))?;

    let service = OpenTdfService::with_kas_url(kas_url);

    // Convert Vec<String> to Vec<&str> for the API
    let value_refs: Vec<&str> = values.iter().map(String::as_str).collect();
    let policy = PolicyBuilder::new()
        .attribute(&namespace, &value_refs)
        .build()
        .context("Failed to build policy")?;

    let manifest = service
        .encrypt(&plaintext, &policy)
        .await
        .context("Encryption failed")?;

    let output_path = output.unwrap_or_else(|| {
        let mut p = input.clone();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        p.set_file_name(format!("{name}.tdf.json"));
        p
    });

    let json = serde_json::to_string_pretty(&manifest)?;
    fs::write(&output_path, json)
        .await
        .with_context(|| format!("Failed to write {}", output_path.display()))?;

    println!("Encrypted to {}", output_path.display());
    println!("  Payload size: {} bytes", manifest.payload.value.len());
    println!("  Policy: {}", manifest.encryption_information.policy);

    Ok(())
}

async fn handle_decrypt(
    input: PathBuf,
    output: Option<PathBuf>,
    client_id: Option<String>,
    client_secret: Option<String>,
) -> Result<()> {
    use arkavo_tdf::{ArkavoKasClient, ArkavoKasConfig, TdfManifest};

    println!("Decrypting {}...", input.display());

    let json = fs::read_to_string(&input)
        .await
        .with_context(|| format!("Failed to read {}", input.display()))?;

    let manifest: TdfManifest =
        serde_json::from_str(&json).context("Failed to parse TDF manifest")?;

    let client_id = client_id.ok_or_else(|| {
        anyhow::anyhow!(
            "OAuth client ID required. Set --client-id or ARKAVO_CLIENT_ID environment variable"
        )
    })?;

    let mut config = ArkavoKasConfig::new(client_id);
    if let Some(secret) = client_secret {
        config = config.with_client_secret(secret);
    }

    let kas_client = ArkavoKasClient::new(config)?;
    let plaintext = kas_client
        .decrypt_manifest(&manifest)
        .await
        .context("Decryption failed")?;

    let output_path = output.unwrap_or_else(|| {
        let mut p = input.clone();
        let name = p.file_name().unwrap().to_string_lossy().to_string();
        let stripped = name
            .strip_suffix(".tdf.json")
            .or_else(|| name.strip_suffix(".tdf"))
            .unwrap_or(&name);
        p.set_file_name(format!("{stripped}.decrypted"));
        p
    });

    fs::write(&output_path, &plaintext)
        .await
        .with_context(|| format!("Failed to write {}", output_path.display()))?;

    println!("Decrypted to {}", output_path.display());
    println!("  Size: {} bytes", plaintext.len());

    Ok(())
}

#[cfg(feature = "iroh")]
async fn handle_stage(input: PathBuf, quiet: bool) -> Result<()> {
    use arkavo_tdf_iroh::{IrohNode, IrohTransport};

    if !quiet {
        println!("Staging {} to Iroh network...", input.display());
    }

    let data = fs::read(&input)
        .await
        .with_context(|| format!("Failed to read {}", input.display()))?;

    let node = IrohNode::memory().await.context("Failed to create Iroh node")?;
    let transport = IrohTransport::new(node);

    let ticket = transport
        .stage_bytes(&data)
        .await
        .context("Failed to stage to Iroh")?;

    if quiet {
        println!("{ticket}");
    } else {
        println!("Staged successfully!");
        println!("  Hash: {}", ticket.hash());
        println!("  Ticket: {ticket}");
        println!();
        println!("Share this ticket to allow others to fetch the data.");
    }

    Ok(())
}

#[cfg(feature = "iroh")]
async fn handle_fetch(ticket_str: String, output: PathBuf) -> Result<()> {
    use arkavo_tdf_iroh::{IrohNode, IrohTicket, IrohTransport};

    println!("Fetching from Iroh network...");

    let ticket: IrohTicket = ticket_str.parse().context("Invalid Iroh ticket")?;

    let node = IrohNode::memory().await.context("Failed to create Iroh node")?;
    let transport = IrohTransport::new(node);

    let data = transport
        .fetch_bytes(&ticket)
        .await
        .context("Failed to fetch from Iroh")?;

    fs::write(&output, &data)
        .await
        .with_context(|| format!("Failed to write {}", output.display()))?;

    println!("Fetched to {}", output.display());
    println!("  Size: {} bytes", data.len());

    Ok(())
}

async fn handle_info(input: PathBuf) -> Result<()> {
    use arkavo_tdf::TdfManifest;

    let json = fs::read_to_string(&input)
        .await
        .with_context(|| format!("Failed to read {}", input.display()))?;

    let manifest: TdfManifest =
        serde_json::from_str(&json).context("Failed to parse TDF manifest")?;

    println!("TDF Manifest: {}", input.display());
    println!();
    println!("Version: {}", manifest.version);
    println!();
    println!("Payload:");
    println!("  Type: {}", manifest.payload.payload_type);
    println!("  MIME: {}", manifest.payload.mime_type);
    println!("  Protocol: {}", manifest.payload.protocol);
    println!(
        "  Size: {} bytes (base64)",
        manifest.payload.value.len()
    );
    println!();
    println!("Encryption:");
    println!(
        "  Type: {}",
        manifest.encryption_information.key_type
    );
    println!(
        "  Algorithm: {}",
        manifest.encryption_information.method.algorithm
    );
    println!(
        "  Streamable: {}",
        manifest.encryption_information.method.is_streamable
    );
    println!();
    println!("Key Access:");
    for (i, ka) in manifest.encryption_information.key_access.iter().enumerate() {
        println!("  [{}] Type: {}", i, ka.access_type);
        println!("      URL: {}", ka.url);
        println!("      Protocol: {}", ka.protocol);
    }
    println!();
    println!("Policy (base64): {}", manifest.encryption_information.policy);

    Ok(())
}

use anyhow::Result;
use clap::Subcommand;
use std::path::PathBuf;
use tokio::fs;

#[derive(Debug, Subcommand)]
pub enum DataflowCommand {
    /// Export a dataflow blueprint to a file
    Export {
        /// Pipeline ID to export
        #[arg(short, long)]
        id: uuid::Uuid,

        /// Output file path
        #[arg(short, long)]
        output: PathBuf,

        /// Compress output with gzip
        #[arg(short, long)]
        compress: bool,
    },

    /// Import a dataflow blueprint from a file
    Import {
        /// Input file path
        #[arg(short, long)]
        input: PathBuf,

        /// Compressed input (gzip)
        #[arg(short, long)]
        compressed: bool,

        /// Start pipeline immediately after import
        #[arg(short, long)]
        start: bool,
    },

    /// List all dataflow pipelines
    List,
}

pub async fn handle_dataflow_command(command: DataflowCommand) -> Result<()> {
    use arkavo_dataflow::{DataflowConfig, DataflowEngine};

    let engine = DataflowEngine::new(DataflowConfig::default());

    match command {
        DataflowCommand::Export {
            id,
            output,
            compress,
        } => {
            println!("Exporting pipeline {id}...");

            let data = if compress {
                engine.export_blueprint_compressed(id)?
            } else {
                engine.export_blueprint(id)?.into_bytes()
            };

            fs::write(&output, data).await?;
            println!("✓ Exported to {}", output.display());
        }

        DataflowCommand::Import {
            input,
            compressed,
            start,
        } => {
            println!("Importing blueprint from {}...", input.display());

            let data = fs::read(&input).await?;

            let pipeline_id = if compressed {
                engine.import_blueprint_compressed(&data)?
            } else {
                let json = String::from_utf8(data)?;
                engine.import_blueprint(&json)?
            };

            println!("✓ Imported pipeline {pipeline_id}");

            if start {
                engine.start_pipeline(pipeline_id).await?;
                println!("✓ Pipeline started");
            }
        }

        DataflowCommand::List => {
            let pipelines = engine.list_pipelines();

            if pipelines.is_empty() {
                println!("No pipelines found");
            } else {
                println!("ID                                   | Name");
                println!("-------------------------------------|------------------------------");
                for (id, name) in pipelines {
                    println!("{id} | {name}");
                }
            }
        }
    }

    Ok(())
}

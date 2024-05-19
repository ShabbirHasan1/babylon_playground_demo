use crate::timeframe::Timeframe;
use chrono::Duration;
use clap::{Args, Parser, Subcommand, ValueEnum};

// Define the main Babylon Compute CLI command with its subcommands
#[derive(Debug, Parser)]
#[command(name = "babylon")]
#[command(about = "Babylon CLI", long_about = "Command-line interface for managing the Babylon application")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

// Define all available subcommands for Babylon CLI
#[derive(Debug, Subcommand)]
pub enum Commands {
    // Run command to start the Babylon application
    #[command(arg_required_else_help = true)]
    Run {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        // The flag to enable or disable the graphical user interface
        #[arg(short = 'g', long, required = false, require_equals = false, help = "Enable or disable the graphical user interface")]
        is_graphical: bool,

        // The flag to enable or disable the graphical user interface
        #[arg(
            short = 'R',
            long,
            required = false,
            require_equals = false,
            help = "Enable or disable the restricted mode (no trading) but unrestricted IP access"
        )]
        is_restricted: bool,

        // The flag to enable or disable the graphical user interface
        #[arg(short = 'f', long, required = false, require_equals = false, help = "Enable or disable the favorite symbols only mode")]
        favorite_only: bool,
    },

    // RunSimulator command to start the Babylon application in simulator mode
    #[command(arg_required_else_help = true)]
    RunPlayground {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,
    },

    // Run test command to start the Babylon application in test mode
    #[command(arg_required_else_help = true)]
    RunExperiment {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,
    },

    // AddCoin command to add a new coin to the configuration
    #[command(arg_required_else_help = true)]
    AddCoin {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        // The name of the coin to add
        #[arg(short, long, required = true, require_equals = false, help = "Name of the coin to add")]
        name: String,

        // Whether the coin is enabled
        #[arg(short, long, required = false, require_equals = false, help = "Whether the coin is enabled")]
        is_enabled: Option<bool>,

        // Whether the coin is a favorite
        #[arg(short, long, required = false, require_equals = false, help = "Whether the coin is a favorite")]
        is_favorite: Option<bool>,

        // The timeframes of the coin
        #[arg(short, long, required = false, require_equals = false, help = "The timeframes of the coin")]
        timeframes: Option<Vec<String>>,
    },

    // DeleteCoin command to delete a coin from the configuration
    #[command(arg_required_else_help = true)]
    DeleteCoin {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        // The name of the coin to delete
        #[arg(short, long, required = true, require_equals = false, help = "Name of the coin to delete")]
        name: String,
    },

    // EditCoin command to edit a coin in the configuration
    #[command(arg_required_else_help = true)]
    EditCoin {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        // The name of the coin to edit
        #[arg(short, long, required = true, require_equals = false, help = "Name of the coin to edit")]
        name: String,

        // The new value for whether the coin is enabled
        #[arg(short, long, required = false, require_equals = false, help = "The new value for whether the coin is enabled")]
        is_enabled: Option<bool>,

        // The new value for whether the coin is a favorite
        #[arg(short, long, required = false, require_equals = false, help = "The new value for whether the coin is a favorite")]
        is_favorite: Option<bool>,

        // The new timeframes of the coin
        #[arg(short, long, required = false, require_equals = false, help = "The new timeframes of the coin")]
        timeframes: Option<Vec<String>>,
    },

    // Inspect command to view the current state of a symbol for a specific timeframe
    #[command(arg_required_else_help = true)]
    Inspect {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        // The symbol to inspect
        #[arg(short, long, required = false, require_equals = false, help = "Symbol to inspect (will inspect all symbols if not specified))")]
        symbol: Option<String>,

        // The timeframes to inspect for the symbol
        #[arg(short = 't', long, required = false, require_equals = false, help = "Timeframe to inspect for the symbol")]
        timeframes: Vec<Timeframe>,

        // The flag to whether or not to inspect all timeframes
        #[arg(short = 'a', long, required = false, require_equals = false, help = "Apply command to all timeframes")]
        all_timeframes: bool,
    },

    #[command(arg_required_else_help = true)]
    InitialFetch {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        // The symbol to fetch initial data for
        #[arg(short, long, required = false, require_equals = false, help = "Symbol to fetch initial data for")]
        symbol: String,

        // The timeframes to update for all symbols
        #[arg(short = 't', long, required = false, require_equals = false, help = "Timeframe to update for the symbol")]
        timeframes: Vec<Timeframe>,

        // The flag to whether or not to update all timeframes
        #[arg(short = 'a', long, required = false, require_equals = false, help = "Apply command to all timeframes")]
        all_timeframes: bool,
    },

    // UpdateFetch command to update the data for a symbol for a specific timeframe
    #[command(arg_required_else_help = true)]
    FetchUpdates {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        // The symbol to fetch updated data for
        #[arg(short, long, required = false, require_equals = false, help = "Symbol to fetch updated data for")]
        symbol: String,

        // The timeframes to update for the symbol
        #[arg(short = 't', long, required = false, require_equals = false, help = "Timeframe to update for the symbol")]
        timeframes: Vec<Timeframe>,

        // The flag to whether or not to update all timeframes
        #[arg(short = 'a', long, required = false, require_equals = false, help = "Apply command to all timeframes")]
        all_timeframes: bool,
    },

    #[deprecated]
    // Update command to update the data for a symbol for a specific timeframe
    #[command(arg_required_else_help = true)]
    Update {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        // The symbol to update
        #[arg(short, long, required = true, require_equals = false, help = "Symbol to update")]
        symbol: String,

        // The timeframes to update for the symbol
        #[arg(short = 't', long, required = false, require_equals = false, help = "Timeframe to update for the symbol")]
        timeframes: Vec<Timeframe>,

        // The flag to whether or not to update all timeframes
        #[arg(short = 'a', long, required = false, require_equals = false, help = "Apply command to all timeframes")]
        all_timeframes: bool,
    },

    // UpdateAll command to update the data for all symbols for a specific timeframe
    // NOTE: If no timeframe is specified, all timeframes (supported by each respective symbol) will be updated
    #[command(arg_required_else_help = true)]
    UpdateAll {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        // The timeframes to update for all symbols
        #[arg(short = 't', long, required = false, require_equals = false, help = "Timeframe to update for the symbol")]
        timeframes: Vec<Timeframe>,

        // The flag to whether or not to update all timeframes
        #[arg(short = 'a', long, required = false, require_equals = false, help = "Apply command to all timeframes")]
        all_timeframes: bool,
    },

    // List command to list all symbols and their respective timeframes
    #[command(arg_required_else_help = true)]
    List {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        #[arg(short, long, required = false, require_equals = false, help = "Flag to show only simple output")]
        simple: bool,

        #[arg(short, long, required = false, require_equals = false, help = "Flag to list only favorite symbols")]
        favorite_only: bool,
    },

    // FixGaps command to fix gaps in the data for a symbol for a specific timeframe
    // NOTE: If no timeframe is specified, all timeframes (supported by each respective symbol) will be updated
    #[command(arg_required_else_help = true)]
    FixGaps {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        // The symbol to fix gaps for
        #[arg(short, long, required = true, require_equals = false, help = "Symbol to fix gaps for")]
        symbol: String,

        // The timeframes to fix gaps for the symbol
        #[arg(short = 't', long, required = false, require_equals = false, help = "Timeframe to fix gaps for the symbol")]
        timeframes: Vec<Timeframe>,

        // The flag to whether or not to fix gaps for all timeframes
        #[arg(short = 'a', long, required = false, require_equals = false, help = "Apply command to all timeframes")]
        all_timeframes: bool,
    },

    // Reset command to reset the data for a symbol for a specific timeframe
    #[command(arg_required_else_help = true)]
    Reset {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        // The symbol to reset
        #[arg(short, long, required = true, require_equals = false, help = "Symbol to reset")]
        symbol: String,

        // The timeframes to reset for the symbol
        #[arg(short = 't', long, required = false, require_equals = false, help = "Timeframes to reset for the symbol")]
        timeframes: Vec<Timeframe>,

        // The flag to whether or not to reset all timeframes
        #[arg(short = 'a', long, required = false, require_equals = false, help = "Apply command to all timeframes")]
        all_timeframes: bool,
    },

    // Reset command to reset the data for a symbol for a specific timeframe
    #[command(arg_required_else_help = true)]
    Rollback {
        // The configuration file path to use for running Babylon
        #[arg(short, long, required = true, require_equals = false, help = "Path to configuration file")]
        config: String,

        // The symbol to rollback
        #[arg(short, long, required = false, require_equals = false, help = "Symbol to rollback")]
        symbol: Option<String>,

        // The timeframes to rollback for the symbol
        #[arg(short = 't', long, required = false, require_equals = false, help = "Timeframes to rollback for the symbol")]
        timeframes: Vec<Timeframe>,

        // The flag to whether or not to rollback all timeframes
        #[arg(short = 'a', long, required = false, require_equals = false, help = "Apply command to all timeframes")]
        all_timeframes: bool,

        // The period to rollback
        #[arg(short = 'p', long, required = false, require_equals = false, help = "Period to rollback")]
        period: String,

        // The flag to whether to rollback from most recent data or NOW
        #[arg(short = 'r', long, required = false, require_equals = false, help = "Rollback starting from most recent data instead of NOW")]
        from_most_recent: bool,
    },
}

use std::path::Path;
use std::vec::Vec;

use anyhow::{anyhow, Context, Error, Result};

use clap::{crate_version, App, Arg};

use reqwest::blocking::Client;

use dialoguer::{Confirm, Input};
use indicatif::MultiProgress;

use sysinfo::{System, SystemExt};

use rayon::prelude::*;

mod download;
mod psa;

//type Error = Box<dyn std::error::Error>;
//type Error = anyhow::Error; <- currently in use

fn main() -> Result<(), Error> {
    env_logger::init();

    let matches = App::new("PSA firmware update.")
        .version(crate_version!())
        .about("CLI alternative to Peugeot/Citroën/Open update for NAC/RCC firmware updates, hopefully more robust. Supports for resume of downloads.")
        .arg(Arg::with_name("VIN")
            .help("Sets the VIN to check for update")
            .required(true)
            .index(1))
        .arg(Arg::with_name("map")
            .help("Sets the map to check for update. Supported maps:\n\
                - afr: Africa\n\
                - alg: Algeria\n\
                - asia: Asia\n\
                - eur: Europe\n\
                - isr: Israel\n\
                - latam: Latin America\n\
                - latam-chile: Latin America Chile\n\
                - mea: Middle East\n\
                - oce: Oceania\n\
                - russia: Russia\n\
                - taiwan: Taiwan")
            .required(false)
            .long("map")
            .takes_value(true))
        .get_matches();

    let vin = matches.value_of("VIN").expect("VIN is missing");
    let map = matches.value_of("map");

    // TODO investigate compression such as gzip for faster download
    let client = Client::builder()
        .build()
        .with_context(|| format!("Failed to create HTTP client"))?;

    let update_response = psa::request_available_updates(&client, vin, map)?;

    if update_response.software.is_none() {
        println!("No update found");
        return Ok(());
    }

    let mut selected_updates: Vec<psa::SoftwareUpdate> = Vec::new();

    for software in update_response.software.unwrap() {
        for update in &software.update {
            // A empty update can be sent by the server when there are no available update
            if !update.update_id.is_empty() {
                psa::print(&software, update);
                if confirm("Download update?")? {
                    selected_updates.push(update.clone());
                }
            }
        }
    }

    if selected_updates.is_empty() {
        return Ok(());
    }

    let multi_progress = MultiProgress::new();

    // Download concurrently
    let downloaded_updates: Result<Vec<psa::DownloadedUpdate>, _> = selected_updates
        .par_iter()
        .map(|update| {
            psa::download_update(&client, update, &multi_progress)
                .with_context(|| format!("Failed to download update"))
        })
        .collect();

    let downloaded_updates = match downloaded_updates {
        Ok(update) => update,
        Err(error) => return Err(error),
    };

    if !confirm(
        "To proceed to extraction of update(s), please insert an empty USB disk formatted as FAT32. Continue?",
    )? {
        return Ok(());
    }

    // Listing available disks for extraction
    let mut sys: System = System::new();
    sys.refresh_disks_list();
    sys.refresh_disks();
    // TODO check destination available space.
    psa::print_disks(&sys);

    let destination = prompt("Location where to extract the update files (IMPORTANT: Should be the root of an EMPTY USB device formatted as FAT32): ")?;
    if destination.is_empty() {
        println!("No location, skipping extraction");
    } else {
        let destination_path = Path::new(&destination);
        if !destination_path.is_dir() {
            return Err(anyhow!(
                "Destination does not exist or is not a directory: {}",
                destination_path.to_string_lossy()
            ));
        }

        for update in downloaded_updates {
            psa::extract_update(&update, destination_path)
                .with_context(|| format!("Failed to extract update"))?;
        }
    }

    Ok(())
}

fn confirm(message: &str) -> Result<bool, Error> {
    Ok(Confirm::new().with_prompt(message).interact()?)
}

fn prompt(message: &str) -> Result<String, Error> {
    Ok(Input::new().with_prompt(message).interact_text()?)
}

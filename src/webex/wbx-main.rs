#![allow(dead_code)]
#![allow(unused_variables)]
#![allow(unused)]

//! # webex-rust
//!
//! A minimal text client for Webex.
//!
//! Current and future functionality:
//! not much yet...
//! - [ ] list rooms
//! - [ ] support rooms in teams
//! - [ ] read messages in selected room
//! - [ ] send message to a room/person
//! - [ ] show rooms that have unread content
//! - [ ] handle @mentions
//!
//! # DISCLAIMER
//!
//! This crate is not maintained by Cisco, and not an official Cisco product. The
//! author is a current employee at Cisco, but has no direct affiliation
//! with the Webex development team.
//!
pub mod auth;
mod sample;
use webex;
use std::io;
use tui::{backend::CrosstermBackend, Terminal};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // let webex = auth::get_webex_client().await;

    // println!("Getting list of rooms");
    // let rooms = webex.get_all_rooms().await.expect("obtaining rooms");
    let rooms = sample::rooms();
    println!("{rooms:#?}");

    // let res : Vec<webex::Team> = webex.list().await.expect("obtaining data");
    // println!("{res:#?}");

    Ok(())
}

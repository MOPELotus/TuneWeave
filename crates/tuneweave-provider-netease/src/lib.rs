mod auth;
mod client;
mod crypto;
mod dto;
mod provider;

pub use auth::{NeteaseAccountSummary, NeteaseCaptchaVerification, NeteaseLoginResult};
pub use client::{
    NeteaseClient, NeteaseConfig, NeteaseQrCheck, NeteaseQrLogin, NeteaseQrState, NeteaseResponse,
};
pub use provider::NeteaseProvider;

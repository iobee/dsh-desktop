use std::{
    env, fs,
    fs::File,
    io::{self, Read},
};

use base64::{engine::general_purpose::STANDARD, Engine};
use minisign_verify::{PublicKey, Signature};

fn decode_wrapper(encoded: &str) -> Result<String, Box<dyn std::error::Error>> {
    Ok(String::from_utf8(STANDARD.decode(encoded.trim())?)?)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut arguments = env::args().skip(1);
    let config_path = arguments.next().ok_or("missing tauri.conf.json path")?;
    let signature_path = arguments.next().ok_or("missing signature path")?;
    let artifact_path = arguments.next().ok_or("missing update artifact path")?;
    if arguments.next().is_some() {
        return Err("unexpected extra argument".into());
    }

    let config: serde_json::Value = serde_json::from_reader(File::open(config_path)?)?;
    let encoded_public_key = config
        .pointer("/plugins/updater/pubkey")
        .and_then(serde_json::Value::as_str)
        .ok_or("updater public key is missing from tauri.conf.json")?;
    let public_key = PublicKey::decode(&decode_wrapper(encoded_public_key)?)?;
    let encoded_signature = fs::read_to_string(signature_path)?;
    let signature = Signature::decode(&decode_wrapper(&encoded_signature)?)?;
    let mut verifier = public_key.verify_stream(&signature)?;
    let mut artifact = File::open(artifact_path)?;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = artifact.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        verifier.update(&buffer[..read]);
    }
    verifier.finalize()?;
    io::Write::write_all(&mut io::stdout(), b"Updater signature verified\n")?;
    Ok(())
}

use std::str::FromStr;

use log::info;
use serde::Deserialize;
use serde::Serialize;
use substrate_api_client::sp_runtime::app_crypto::sr25519::Public;
use tokio::task::spawn_blocking;
use tokio::time::{sleep, Duration};
use web3::block_on;

use crate::database::ScannerState;
use crate::js_call::{self, get_fee};

// MEJORAS GLITCH:

// - ASINCRONISMO EN TRANSFERENCIAS EN GLITCH
// - ESTADO INTERMEDIO
// - FLOTANTE PARA EL FEE

pub async fn fee_payer(
    scanner_state: ScannerState,
    interval_in_days: u8,
    glitch_pk: Option<String>,
    fee_address: String,
) {
    loop {
        sleep(Duration::from_secs(u64::from(interval_in_days) * 86400)).await;

        let fee_to_send = scanner_state.get_fee_counter().await;

        if fee_to_send != 0 {
            let hash_result = js_call::transfer(fee_to_send, fee_address.as_str());

            match hash_result {
                Ok(xt_hash) => {
                    scanner_state
                        .insert_tx_fee(xt_hash, fee_to_send.to_string())
                        .await;
                    info!(
                        "The transfer of the business fee ({}) has been completed",
                        fee_to_send
                    );
                }
                Err(e) => {
                    info!(
                        "Transfer of the business fee not completed. It will be tried again.: {}",
                        e
                    );
                }
            }
        }
    }
}

pub async fn transfer(
    scanner_state: ScannerState,
    glitch_pk: Option<String>,
    node_glitch: String,
    business_fee: u128,
    glitch_gas: bool,
) {
    // This is now done from Javascript.
    /*
    let seed: [u8; 32] = decode(glitch_pk.unwrap()).unwrap().try_into().unwrap();
    let pair = Pair::from_seed(&seed);
    let client = WsRpcClient::new(node_glitch.as_str()).unwrap();
    let mut api =
        Api::<_, _, AssetTipExtrinsicParams<Runtime>, Runtime>::new(client.clone()).unwrap();
    api.set_signer(pair);
    */

    loop {
        let txs = scanner_state.txs_to_process().await;

        for tx in txs {
            match Public::from_str(&tx.glitch_address) {
                Ok(_) => (),
                Err(error) => {
                    scanner_state
                        .save_with_error(tx.id, format!("Error with address: {error:?}"))
                        .await;
                    continue;
                }
            };

            let amount = match tx.amount.clone().parse::<u128>() {
                Ok(a) => a,
                Err(error) => {
                    scanner_state
                        .save_with_error(tx.id, format!("Error with amount: {error:?}"))
                        .await;
                    continue;
                }
            };

            let fee = if glitch_gas {
                get_fee(amount, &tx.glitch_address)
            } else {
                0_u128
            };

            let amount_to_transfer = amount - fee;
            let business_fee_amount = (amount_to_transfer / 100) * business_fee;

            let fee_counter = scanner_state.get_fee_counter().await;

            scanner_state
                .modify_fee_counter(fee_counter + business_fee_amount)
                .await;

            info!("Business fee amount is: {}", business_fee_amount);

            info!("Amount received in the ETH transaction: {}", amount);
            info!(
                "Estimated fee for the transaction on the Glitch network {}",
                fee
            );
            info!("Amount to be transferred {}", amount_to_transfer);

            let scanner_state_clone = scanner_state.clone();

            scanner_state.update_tx_to_processing(tx.id).await;

            spawn_blocking(move || {
                block_on(async {
                    let xt_result = js_call::transfer(
                        amount_to_transfer - business_fee_amount,
                        &tx.glitch_address,
                    );

                    match xt_result {
                        Ok(xt_hash) => {
                            scanner_state_clone
                                .update_tx(tx.id, xt_hash, business_fee_amount, business_fee)
                                .await;

                            info!("Trasfer to address {} completed!", tx.glitch_address);
                        }
                        Err(e) => info!(
                            "Transfer to address {} not completed with error {}. It will be tried again.",
                            tx.glitch_address,
                            e
                        ),
                    }
                });
            });
        }

        sleep(Duration::from_millis(5000)).await;
    }
}

#[derive(Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeeResult {
    pub class: String,
    pub partial_fee: String,
    pub weight: u128,
}

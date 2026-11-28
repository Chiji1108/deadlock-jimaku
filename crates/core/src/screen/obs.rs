//! OBS のソースに映っているゲームの画面を、OBS の WebSocket サーバーからもらう（DESIGN.md 18.8）。
//! ゲーム用と配信用の PC を分けている人は、ゲームの画面がキャプチャーボードから OBS に入っている。
//! キャプチャーボードは OBS が開いているので、アプリは自分では開かず、OBS に「このソースの今の絵を 1 枚」と頼む。
//!
//! 同じ PC の OBS にだけつなぐ（`127.0.0.1`）。絵は JPEG でもらう。PNG は、OBS が 1 枚を固めるのに
//! 0.4〜1.2 秒かかり、1 秒に 3 回に間に合わない（JPEG は 40 ミリ秒ほど。読み取りの結果は変わらなかった）。
//! 呼び出しは待つので、専用のスレッドから呼ぶ。

use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use image::RgbImage;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::{Error, Message, WebSocket, client};

use crate::types::ScreenStatus;

/// OBS の WebSocket サーバーの、はじめのポート
pub const PORT: u16 = 4455;
/// つなぐのを待つ長さ。同じ PC なので、開いていればすぐつながる
const CONNECT: Duration = Duration::from_secs(1);
/// 返事を待つ長さ
const ANSWER: Duration = Duration::from_secs(5);
/// JPEG の品質。90 で、読み取りの結果が元の絵と変わらなかった
const QUALITY: u8 = 90;
/// 認証に失敗したときに、OBS が接続を閉じる番号
const AUTHENTICATION_FAILED: u16 = 4009;
/// 頼んだもの（ソース）が見つからないときの番号
const RESOURCE_NOT_FOUND: u64 = 600;

/// OBS から絵をもらえない理由
#[derive(Debug)]
pub enum Refused {
    /// つながらない（OBS が開いていないか、WebSocket サーバーが有効でない）
    Closed,
    /// パスワードが違うか、入れていない
    Password,
    /// その名前のソースがない
    NoInput,
    Other(anyhow::Error),
}

impl<E: Into<anyhow::Error>> From<E> for Refused {
    fn from(error: E) -> Self {
        Self::Other(error.into())
    }
}

/// 操作パネルに見せる理由にする
impl From<Refused> for ScreenStatus {
    fn from(refused: Refused) -> Self {
        match refused {
            Refused::Closed => Self::NoObs,
            Refused::Password => Self::ObsPassword,
            Refused::NoInput => Self::NoInput,
            Refused::Other(e) => Self::Failed {
                detail: format!("{e:#}"),
            },
        }
    }
}

/// OBS とつながっている口。落とすと、接続を閉じる
pub struct Obs {
    socket: WebSocket<TcpStream>,
    requests: u64,
}

impl Obs {
    pub fn connect(port: u16, password: Option<&str>) -> Result<Self, Refused> {
        let address = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let stream = TcpStream::connect_timeout(&address, CONNECT).map_err(|_| Refused::Closed)?;
        stream.set_read_timeout(Some(ANSWER))?;
        stream.set_write_timeout(Some(ANSWER))?;
        // そのポートで別のものが待っていた
        let (socket, _) = client(format!("ws://{address}"), stream).map_err(|_| Refused::Closed)?;
        let mut obs = Self {
            socket,
            requests: 0,
        };
        let hello = obs.receive(0)?;
        let authentication = match &hello["authentication"] {
            Value::Null => None,
            asked => {
                let password = password.ok_or(Refused::Password)?;
                let (salt, challenge) = (asked["salt"].as_str(), asked["challenge"].as_str());
                let (salt, challenge) = salt
                    .zip(challenge)
                    .ok_or_else(|| anyhow::anyhow!("OBS の認証の問いが読めない"))?;
                Some(answer(password, salt, challenge))
            }
        };
        // 出来事の知らせは要らない（eventSubscriptions: 0）
        obs.send(
            1,
            json!({ "rpcVersion": 1, "authentication": authentication, "eventSubscriptions": 0 }),
        )?;
        obs.receive(2)?;
        Ok(obs)
    }

    /// OBS のソース（入力）の名前
    pub fn inputs(&mut self) -> Result<Vec<String>, Refused> {
        let listed = self.request("GetInputList", json!({}))?;
        let inputs = listed["inputs"].as_array().into_iter().flatten();
        Ok(inputs
            .filter_map(|input| input["inputName"].as_str().map(str::to_owned))
            .collect())
    }

    /// ソースの今の絵（ソースのそのままの大きさ）
    pub fn screenshot(&mut self, input: &str) -> Result<RgbImage, Refused> {
        // まだ選んでいない
        if input.is_empty() {
            return Err(Refused::NoInput);
        }
        let shot = self.request(
            "GetSourceScreenshot",
            json!({ "sourceName": input, "imageFormat": "jpg", "imageCompressionQuality": QUALITY }),
        )?;
        // `data:image/jpg;base64,…`
        let data = (shot["imageData"].as_str())
            .and_then(|data| data.split_once(',').map(|(_, data)| data))
            .ok_or_else(|| anyhow::anyhow!("OBS の絵が読めない"))?;
        let jpeg = STANDARD.decode(data)?;
        Ok(image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg)?.into_rgb8())
    }

    fn request(&mut self, kind: &str, data: Value) -> Result<Value, Refused> {
        self.requests += 1;
        let id = self.requests.to_string();
        self.send(
            6,
            json!({ "requestType": kind, "requestId": id, "requestData": data }),
        )?;
        loop {
            let answer = self.receive(7)?;
            if answer["requestId"] != id.as_str() {
                continue;
            }
            let status = &answer["requestStatus"];
            return if status["result"] == true {
                Ok(answer["responseData"].clone())
            } else if status["code"] == RESOURCE_NOT_FOUND {
                Err(Refused::NoInput)
            } else {
                Err(Refused::Other(anyhow::anyhow!(
                    "OBS が {kind} を断った（{}）",
                    status["comment"].as_str().unwrap_or_default()
                )))
            };
        }
    }

    fn send(&mut self, op: u8, data: Value) -> Result<(), Refused> {
        let message = json!({ "op": op, "d": data }).to_string();
        Ok(self.socket.send(Message::text(message))?)
    }

    /// `op` の知らせが届くまで読んで、その中身を返す
    fn receive(&mut self, op: u8) -> Result<Value, Refused> {
        loop {
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    let message: Value = serde_json::from_str(&text)?;
                    if message["op"] == op {
                        return Ok(message["d"].clone());
                    }
                }
                Ok(Message::Close(Some(frame)))
                    if frame.code == CloseCode::from(AUTHENTICATION_FAILED) =>
                {
                    return Err(Refused::Password);
                }
                Ok(Message::Close(_)) | Err(Error::ConnectionClosed | Error::AlreadyClosed) => {
                    return Err(Refused::Closed);
                }
                Ok(_) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl Drop for Obs {
    fn drop(&mut self) {
        // 黙って切ると、OBS のログに「途中で切れた」と残る
        let _ = self.socket.close(None);
        let _ = self.socket.flush();
    }
}

/// 認証の答え。パスワードそのものは送らない（obs-websocket 5 の決まり）
fn answer(password: &str, salt: &str, challenge: &str) -> String {
    let hash = |text: String| STANDARD.encode(Sha256::digest(text));
    hash(hash(format!("{password}{salt}")) + challenge)
}

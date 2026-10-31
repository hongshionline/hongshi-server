//! 腾讯云邮件推送（SES）发信模块。
//!
//! 腾讯云没有官方 Rust SDK；为避免引入不可信的第三方 SDK，本模块完全按照官方文档
//! 自行实现 TC3-HMAC-SHA256 签名，只依赖通用的 HMAC / SHA-256 基础库。
//!
//! 参考资料：
//! - 签名方法 v3：<https://cloud.tencent.com/document/product/884/30657>
//! - SendEmail 接口：<https://cloud.tencent.com/document/product/1288/51034>
//!
//! 注意：
//! - 腾讯云 SES 默认只支持**模板发信**（Simple 方式已废弃），因此本模块通过
//!   `Template` 参数发送验证码等触发类邮件。
//! - 所有错误信息都不会包含 SecretId / SecretKey。
//!
//! # 示例
//!
//! ```no_run
//! use hongshi_server::send_email::{send_email, SendEmailRequest};
//!
//! # async fn example() -> Result<(), String> {
//! let request = SendEmailRequest {
//!     from_email_address: "verify@email.hongshi.site".to_string(),
//!     destination: vec!["user@example.com".to_string()],
//!     subject: "邮箱验证码".to_string(),
//!     template_id: 62793,
//!     template_data: r#"{"code":"123456"}"#.to_string(),
//!     reply_to_addresses: None,
//!     trigger_type: None,
//! };
//!
//! let response = send_email("你的 SecretId", "你的 SecretKey", "ap-guangzhou", &request).await?;
//! println!("MessageId = {}, RequestId = {}", response.message_id, response.request_id);
//! # Ok(())
//! # }
//! ```

use std::sync::OnceLock;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// 腾讯云 SES 的接口域名，同时作为签名中的 host。
const HOST: &str = "ses.tencentcloudapi.com";
/// 腾讯云产品名，用于 CredentialScope（date/service/tc3_request）。
const SERVICE: &str = "ses";
/// SendEmail 接口版本。
const VERSION: &str = "2020-10-02";
/// SendEmail 接口名。
const ACTION: &str = "SendEmail";
/// TC3 签名算法名。
const ALGORITHM: &str = "TC3-HMAC-SHA256";
/// 参与签名的请求头列表（官方 SDK 只签 content-type 和 host）。
const SIGNED_HEADERS: &str = "content-type;host";
/// 请求 Content-Type，必须与实际发出的请求头完全一致，否则签名校验失败。
const CONTENT_TYPE: &str = "application/json";

pub const SEND_LOCATION: &str = "ap-guangzhou";

type HmacSha256 = Hmac<Sha256>;

/// 发送模板邮件所需的参数。
#[derive(Debug, Clone)]
pub struct SendEmailRequest {
    /// 发件地址，例如 `verify@email.hongshi.site`（需要在 SES 控制台验证通过）。
    /// 也支持别名格式：`别名 <邮箱地址>`，别名中不能带有冒号。
    pub from_email_address: String,
    /// 收件人地址列表，单次最多 50 个。
    pub destination: Vec<String>,
    /// 邮件主题。
    pub subject: String,
    /// 模板 ID（SES 控制台中已审核通过的模板）。
    pub template_id: i64,
    /// 模板变量参数，JSON 字符串，例如 `{"code":"123456"}`。
    pub template_data: String,
    /// 回复地址，可选；不填时收件人回复邮件会失败。
    pub reply_to_addresses: Option<String>,
    /// 触发类型：0=非触发类，1=触发类（验证码等即时邮件）。不填时默认 1。
    pub trigger_type: Option<i32>,
}

/// SendEmail 接口成功时的响应。
#[derive(Debug, Clone, Deserialize)]
pub struct SendEmailResponse {
    /// 腾讯云为这封邮件生成的唯一标识。
    #[serde(rename = "MessageId")]
    pub message_id: String,
    /// 腾讯云请求 ID，排查问题时需要提供给腾讯云。
    #[serde(rename = "RequestId")]
    pub request_id: String,
}

/// 发送一封模板邮件。
///
/// - `secret_id` / `secret_key`：腾讯云 API 密钥。
/// - `region`：SES 目前仅支持 `ap-guangzhou`（广州）和 `ap-hongkong`（香港）。
///
/// 成功返回 [`SendEmailResponse`]；失败返回可读的错误字符串（不含密钥）。
pub async fn send_email(
    secret_id: &str,
    secret_key: &str,
    region: &str,
    request: &SendEmailRequest,
) -> Result<SendEmailResponse, String> {
    if request.from_email_address.trim().is_empty() {
        return Err("发件地址不能为空".to_string());
    }
    if request.destination.is_empty() {
        return Err("收件人列表不能为空".to_string());
    }
    if request.subject.trim().is_empty() {
        return Err("邮件主题不能为空".to_string());
    }
    if request.template_id <= 0 {
        return Err("模板 ID 必须为正整数".to_string());
    }
    if serde_json::from_str::<serde_json::Value>(&request.template_data).is_err() {
        return Err("template_data 必须是合法的 JSON 字符串".to_string());
    }

    let payload = build_payload(request)?;
    let value = call_api(secret_id, secret_key, region, ACTION, VERSION, &payload).await?;
    parse_send_email_response(&value)
}

/// 组装 SendEmail 的 JSON 请求体。
///
/// 注意：这里的序列化结果就是参与签名的 payload，发送请求时必须使用同一个字符串，
/// 否则签名会校验失败。
fn build_payload(request: &SendEmailRequest) -> Result<String, String> {
    let body = SendEmailBody {
        from_email_address: request.from_email_address.as_str(),
        destination: request.destination.as_slice(),
        subject: request.subject.as_str(),
        template: TemplateBody {
            template_id: request.template_id,
            template_data: request.template_data.as_str(),
        },
        reply_to_addresses: request.reply_to_addresses.as_deref(),
        trigger_type: request.trigger_type.unwrap_or(1),
    };

    serde_json::to_string(&body).map_err(|e| format!("序列化 SendEmail 请求失败: {e}"))
}

/// SendEmail 请求体（字段名与腾讯云 JSON 接口保持一致）。
#[derive(Serialize)]
struct SendEmailBody<'a> {
    #[serde(rename = "FromEmailAddress")]
    from_email_address: &'a str,
    #[serde(rename = "Destination")]
    destination: &'a [String],
    #[serde(rename = "Subject")]
    subject: &'a str,
    #[serde(rename = "Template")]
    template: TemplateBody<'a>,
    #[serde(
        rename = "ReplyToAddresses",
        skip_serializing_if = "Option::is_none"
    )]
    reply_to_addresses: Option<&'a str>,
    #[serde(rename = "TriggerType")]
    trigger_type: i32,
}

/// 模板参数。
#[derive(Serialize)]
struct TemplateBody<'a> {
    #[serde(rename = "TemplateID")]
    template_id: i64,
    #[serde(rename = "TemplateData")]
    template_data: &'a str,
}

/// 调用腾讯云 API（公共逻辑：签名、发送、解析错误）。
async fn call_api(
    secret_id: &str,
    secret_key: &str,
    region: &str,
    action: &str,
    version: &str,
    payload: &str,
) -> Result<serde_json::Value, String> {
    let timestamp = unix_timestamp();
    let authorization = build_authorization(secret_id, secret_key, HOST, payload, timestamp);

    let response = http_client()
        .post(format!("https://{HOST}"))
        .header("Authorization", authorization)
        .header("Content-Type", CONTENT_TYPE)
        .header("X-TC-Action", action)
        .header("X-TC-Timestamp", timestamp.to_string())
        .header("X-TC-Version", version)
        .header("X-TC-Region", region)
        .body(payload.to_string())
        .send()
        .await
        .map_err(|e| format!("请求腾讯云接口失败: {e}"))?;

    let status = response.status();
    let text = response
        .text()
        .await
        .map_err(|e| format!("读取腾讯云接口响应失败: {e}"))?;

    if !status.is_success() {
        return Err(format!("腾讯云接口 HTTP 错误 {status}: {text}"));
    }

    let value: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("解析腾讯云接口响应失败: {e}"))?;

    // 腾讯云业务错误也是 HTTP 200，错误信息在 Response.Error 中。
    if let Some(message) = api_error_message(&value) {
        return Err(message);
    }

    Ok(value)
}

/// 从腾讯云响应中提取业务错误信息。
fn api_error_message(value: &serde_json::Value) -> Option<String> {
    let error = value.pointer("/Response/Error")?;
    let code = error
        .get("Code")
        .and_then(|v| v.as_str())
        .unwrap_or("UnknownError");
    let message = error
        .get("Message")
        .and_then(|v| v.as_str())
        .unwrap_or("未知错误");
    let request_id = value
        .pointer("/Response/RequestId")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    Some(format!(
        "腾讯云接口返回错误: {code}: {message} (RequestId: {request_id})"
    ))
}

/// 解析 SendEmail 成功响应。
fn parse_send_email_response(value: &serde_json::Value) -> Result<SendEmailResponse, String> {
    let response = value
        .get("Response")
        .ok_or_else(|| "腾讯云响应缺少 Response 字段".to_string())?;
    serde_json::from_value::<SendEmailResponse>(response.clone())
        .map_err(|e| format!("解析 SendEmail 响应失败: {e}"))
}

/// 生成 TC3-HMAC-SHA256 的 Authorization 请求头。
///
/// `host` 参数单独传入是为了方便单元测试；生产代码固定使用 [`HOST`]。
fn build_authorization(
    secret_id: &str,
    secret_key: &str,
    host: &str,
    payload: &str,
    timestamp: u64,
) -> String {
    let date = utc_date(timestamp);
    let payload_hash = sha256_hex(payload.as_bytes());

    let canonical_headers = format!("content-type:{CONTENT_TYPE}\nhost:{host}\n");
    let canonical_request = build_canonical_request(&canonical_headers, SIGNED_HEADERS, &payload_hash);
    let canonical_request_hash = sha256_hex(canonical_request.as_bytes());

    let credential_scope = format!("{date}/{SERVICE}/tc3_request");
    let string_to_sign =
        format!("{ALGORITHM}\n{timestamp}\n{credential_scope}\n{canonical_request_hash}");

    // 派生签名密钥：SecretDate -> SecretService -> SecretSigning
    let secret_date = hmac_sha256(format!("TC3{secret_key}").as_bytes(), date.as_bytes());
    let secret_service = hmac_sha256(&secret_date, SERVICE.as_bytes());
    let secret_signing = hmac_sha256(&secret_service, b"tc3_request");
    let signature = hex::encode(hmac_sha256(&secret_signing, string_to_sign.as_bytes()));

    format!(
        "{ALGORITHM} Credential={secret_id}/{credential_scope}, \
         SignedHeaders={SIGNED_HEADERS}, Signature={signature}"
    )
}

/// 拼接规范请求串（CanonicalRequest）。
fn build_canonical_request(
    canonical_headers: &str,
    signed_headers: &str,
    payload_hash: &str,
) -> String {
    format!("POST\n/\n\n{canonical_headers}\n{signed_headers}\n{payload_hash}")
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut mac = <HmacSha256 as Mac>::new_from_slice(key).expect("HMAC 支持任意长度密钥");
    mac.update(data);
    mac.finalize().into_bytes().to_vec()
}

fn sha256_hex(data: &[u8]) -> String {
    hex::encode(Sha256::digest(data))
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or(0)
}

/// 把 Unix 时间戳换算成 UTC 日期（`YYYY-MM-DD`）。
///
/// 签名的 `CredentialScope` 必须使用 UTC 日期，不能用本地时区。
/// 这里使用 Howard Hinnant 的 `civil_from_days` 算法，避免额外依赖。
fn utc_date(timestamp: u64) -> String {
    let days = (timestamp / 86_400) as i64;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if month <= 2 { year + 1 } else { year };
    format!("{year:04}-{month:02}-{day:02}")
}

/// 复用同一个 HTTP 连接池，避免每次发信都重新建立连接。
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()
            .unwrap_or_else(|_| reqwest::Client::new())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与官方 Python SDK 生成 oracle 时完全一致的紧凑请求体。
    const ORACLE_PAYLOAD: &str = r#"{"FromEmailAddress":"verify@email.hongshi.site","Destination":["test@example.com"],"Subject":"邮箱验证码","Template":{"TemplateID":62793,"TemplateData":"{\"code\":\"123456\"}"},"TriggerType":1}"#;
    /// 上面请求体用官方 SDK 在固定时间戳 1551113065 下算出的 Authorization。
    const ORACLE_AUTHORIZATION: &str = "TC3-HMAC-SHA256 Credential=AKIDTESTEXAMPLE0000000000000000/2019-02-25/ses/tc3_request, SignedHeaders=content-type;host, Signature=4fd4150b6c83731fe0856acd0e885d2357cf1fab7c5081690c795914543300cb";

    fn test_request() -> SendEmailRequest {
        SendEmailRequest {
            from_email_address: "verify@email.hongshi.site".to_string(),
            destination: vec!["test@example.com".to_string()],
            subject: "邮箱验证码".to_string(),
            template_id: 62793,
            template_data: r#"{"code":"123456"}"#.to_string(),
            reply_to_addresses: None,
            trigger_type: Some(1),
        }
    }

    #[test]
    fn request_body_matches_official_sdk_vector() {
        let payload = build_payload(&test_request()).unwrap();
        assert_eq!(payload, ORACLE_PAYLOAD);
    }

    #[test]
    fn authorization_matches_official_python_sdk() {
        let authorization = build_authorization(
            "AKIDTESTEXAMPLE0000000000000000",
            "TEST_SECRET_KEY_EXAMPLE",
            HOST,
            ORACLE_PAYLOAD,
            1_551_113_065,
        );
        assert_eq!(authorization, ORACLE_AUTHORIZATION);
    }

    #[test]
    fn utc_date_uses_utc_timezone() {
        assert_eq!(utc_date(0), "1970-01-01");
        assert_eq!(utc_date(86_400), "1970-01-02");
        assert_eq!(utc_date(951_782_400), "2000-02-29");
        // 腾讯云签名文档中的示例时间戳，东八区已是 2019-02-26，但签名必须用 UTC 的 2019-02-25。
        assert_eq!(utc_date(1_551_113_065), "2019-02-25");
        assert_eq!(utc_date(1_700_000_000), "2023-11-14");
    }

    #[test]
    fn canonical_request_hash_matches_tencent_doc_example() {
        // 腾讯云《签名方法 v3》官方示例（cvm DescribeInstances）。
        // 注意 payload 中的中文使用 \u 转义形式，与官方示例保持一致。
        let payload = r#"{"Limit": 1, "Filters": [{"Values": ["\u672a\u547d\u540d"], "Name": "instance-name"}]}"#;
        let payload_hash = sha256_hex(payload.as_bytes());
        assert_eq!(
            payload_hash,
            "35e9c5b0e3ae67532d3c9f17ead6c90222632e5b1ff7f6e89887f1398934f064"
        );

        let canonical_headers = "content-type:application/json; charset=utf-8\n\
                                 host:cvm.tencentcloudapi.com\n\
                                 x-tc-action:describeinstances\n";
        let canonical_request = build_canonical_request(
            canonical_headers,
            "content-type;host;x-tc-action",
            &payload_hash,
        );
        assert_eq!(
            sha256_hex(canonical_request.as_bytes()),
            "7019a55be8395899b900fb5564e4200d984910f34794a27cb3fb7d10ff6a1e84"
        );
    }

    #[test]
    fn parses_success_response() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{"Response":{"MessageId":"qcloud-ses-messageid","RequestId":"8979fc1e-9564-4fc9-bf7d-2958ce679b72"}}"#,
        )
        .unwrap();
        let response = parse_send_email_response(&value).unwrap();
        assert_eq!(response.message_id, "qcloud-ses-messageid");
        assert_eq!(response.request_id, "8979fc1e-9564-4fc9-bf7d-2958ce679b72");
    }

    #[test]
    fn detects_tencent_business_error() {
        let value: serde_json::Value = serde_json::from_str(
            r#"{"Response":{"Error":{"Code":"AuthFailure.SignatureFailure","Message":"签名错误"},"RequestId":"abc"}}"#,
        )
        .unwrap();
        assert_eq!(
            api_error_message(&value).unwrap(),
            "腾讯云接口返回错误: AuthFailure.SignatureFailure: 签名错误 (RequestId: abc)"
        );
    }

    /// 手动集成测试：读取 config.json 中的真实密钥，调用只读接口验证签名是否被腾讯云接受。
    /// 不会发送任何邮件。
    ///
    /// 运行：`cargo test -- --ignored`
    #[tokio::test]
    #[ignore = "需要 config.json 中的真实密钥并联网，手动运行：cargo test -- --ignored"]
    async fn real_tencent_api_accepts_our_signature() {
        let config_text =
            std::fs::read_to_string("config.json").expect("读取 config.json 失败（请在项目根目录运行测试）");
        let config: serde_json::Value =
            serde_json::from_str(&config_text).expect("config.json 不是合法的 JSON");
        let secret_id = config
            .pointer("/send_email/secret_id")
            .and_then(|v| v.as_str())
            .expect("config.json 缺少 send_email.secret_id");
        let secret_key = config
            .pointer("/send_email/secret_key")
            .and_then(|v| v.as_str())
            .expect("config.json 缺少 send_email.secret_key");

        // ListEmailAddress 是只读接口，用来验证签名，不会发送邮件。
        let value = call_api(
            secret_id,
            secret_key,
            "ap-guangzhou",
            "ListEmailAddress",
            VERSION,
            "{}",
        )
        .await
        .expect("调用腾讯云只读接口失败：签名错误或网络异常");

        let sender_count = value
            .pointer("/Response/EmailSenders")
            .and_then(|v| v.as_array())
            .map(|senders| senders.len())
            .unwrap_or(0);
        println!("腾讯云只读接口调用成功，签名有效，已配置发信地址数量: {sender_count}");
        assert!(value.get("Response").is_some());
    }
}

//! Pictures and files: what's kept to fetch a received one later, and the messages that carry one
//! an agent sends

use base64::prelude::{BASE64_STANDARD, Engine as _};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;
use whatsapp_rust::{download::MediaType, prelude::wa, upload::UploadResponse};

use super::inbox::Kind;
use crate::crypto::Cipher;

/// The largest file an agent may open or send. Base64 makes an opened one about a third larger in
/// the answer.
pub const FILE_MAX: usize = 16 * 1024 * 1024;

/// What WhatsApp needs to hand over a received file again: where it is and the key that opens it.
/// Kept inside the sealed body, never in the clear.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaRef {
    pub direct_path: String,
    /// Base64
    pub media_key: String,
    /// Base64
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_sha256: Option<String>,
    /// Base64
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_enc_sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_length: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
}

impl MediaRef {
    /// The reference a message's picture, video, voice note, file or sticker carries; `None` for
    /// any other message, or one whose file can't be fetched
    #[must_use]
    pub fn of(message: &wa::Message) -> Option<Self> {
        let encode = |bytes: Option<&Vec<u8>>| bytes.map(|bytes| BASE64_STANDARD.encode(bytes));
        macro_rules! reference {
            ($media:expr) => {{
                let media = $media;
                Some(Self {
                    direct_path: media.direct_path.clone().filter(|path| !path.is_empty())?,
                    media_key: encode(media.media_key.as_ref())?,
                    file_sha256: encode(media.file_sha256.as_ref()),
                    file_enc_sha256: encode(media.file_enc_sha256.as_ref()),
                    file_length: media.file_length,
                    mime_type: media.mimetype.clone(),
                })
            }};
        }
        if let Some(image) = message.image_message.as_option() {
            reference!(image)
        } else if let Some(video) = message.video_message.as_option() {
            reference!(video)
        } else if let Some(audio) = message.audio_message.as_option() {
            reference!(audio)
        } else if let Some(document) = message.document_message.as_option() {
            reference!(document)
        } else if let Some(sticker) = message.sticker_message.as_option() {
            reference!(sticker)
        } else {
            None
        }
    }

    /// A message the library can download from, of the kind the kept message was
    ///
    /// # Errors
    /// A stored key or hash isn't base64, or the kind carries no file.
    pub fn downloadable(&self, kind: Kind) -> Result<Box<dyn Downloadable>, &'static str> {
        let decode = |text: &str| {
            BASE64_STANDARD
                .decode(text)
                .map_err(|_| "a stored key is damaged")
        };
        let key = decode(&self.media_key)?;
        let sha = self.file_sha256.as_deref().map(decode).transpose()?;
        let enc = self.file_enc_sha256.as_deref().map(decode).transpose()?;
        let path = Some(self.direct_path.clone());
        macro_rules! message {
            ($type:ty) => {{
                let mut media = <$type>::default();
                media.direct_path = path;
                media.media_key = Some(key);
                media.file_sha256 = sha;
                media.file_enc_sha256 = enc;
                media.file_length = self.file_length;
                media.mimetype = self.mime_type.clone();
                Box::new(media) as Box<dyn Downloadable>
            }};
        }
        Ok(match kind {
            Kind::Image => message!(wa::message::ImageMessage),
            Kind::Video => message!(wa::message::VideoMessage),
            Kind::Audio => message!(wa::message::AudioMessage),
            Kind::Document => message!(wa::message::DocumentMessage),
            Kind::Sticker => message!(wa::message::StickerMessage),
            _ => return Err("this message has no file"),
        })
    }

    /// The media key, as asking the phone to upload the file again needs it
    #[must_use]
    pub fn key(&self) -> Option<[u8; 32]> {
        BASE64_STANDARD
            .decode(&self.media_key)
            .ok()?
            .try_into()
            .ok()
    }
}

pub use whatsapp_rust::wacore::download::Downloadable;

/// What a file an agent sends goes out as
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[sqlx(type_name = "text", rename_all = "lowercase")]
#[serde(rename_all = "snake_case")]
pub enum FileKind {
    Image,
    Video,
    Audio,
    Document,
}

impl FileKind {
    /// A picture WhatsApp shows inline only as JPEG or PNG, a video only as MP4; anything else
    /// goes as a file to download
    #[must_use]
    pub fn of(mime_type: &str) -> Self {
        match mime_type {
            "image/jpeg" | "image/png" => Self::Image,
            "video/mp4" | "video/3gpp" => Self::Video,
            "audio/ogg" | "audio/mpeg" | "audio/mp4" | "audio/aac" | "audio/amr" => Self::Audio,
            _ => Self::Document,
        }
    }

    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Document => "document",
        }
    }

    #[must_use]
    pub const fn media_type(self) -> MediaType {
        match self {
            Self::Image => MediaType::Image,
            Self::Video => MediaType::Video,
            Self::Audio => MediaType::Audio,
            Self::Document => MediaType::Document,
        }
    }
}

/// What a send's picture or file is, for the owner's pages and the approval it waits in
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct FileNote {
    pub id: Uuid,
    pub kind: FileKind,
    pub mime_type: String,
    #[schema(required = true)]
    pub file_name: Option<String>,
    pub size: u64,
}

fn file_context(id: Uuid) -> Vec<u8> {
    [b"outgoing:".as_slice(), id.as_bytes()].concat()
}

/// # Errors
/// The system random number generator failed.
pub fn seal_file(cipher: &Cipher, id: Uuid, data: &[u8]) -> anyhow::Result<Vec<u8>> {
    cipher.encrypt(data, &file_context(id))
}

/// # Errors
/// The data didn't open: another key, another file, or altered.
pub fn open_file(cipher: &Cipher, id: Uuid, sealed: &[u8]) -> anyhow::Result<Vec<u8>> {
    Ok(cipher.decrypt(sealed, &file_context(id))?.to_vec())
}

impl From<FileKind> for Kind {
    fn from(kind: FileKind) -> Self {
        match kind {
            FileKind::Image => Self::Image,
            FileKind::Video => Self::Video,
            FileKind::Audio => Self::Audio,
            FileKind::Document => Self::Document,
        }
    }
}

/// A file on its way out
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutgoingFile {
    pub kind: FileKind,
    pub mime_type: String,
    pub file_name: Option<String>,
    pub data: Vec<u8>,
}

/// The message that carries an uploaded file. A caption goes with a picture, video or file; a
/// voice note or audio can't carry one.
#[must_use]
pub fn file_message(
    file: &OutgoingFile,
    upload: UploadResponse,
    caption: Option<String>,
    context: Option<wa::ContextInfo>,
) -> wa::Message {
    let caption = caption.filter(|caption| !caption.is_empty());
    let mut message = wa::Message::default();
    macro_rules! fill {
        ($media:ident) => {{
            $media.url = Some(upload.url);
            $media.direct_path = Some(upload.direct_path);
            $media.media_key = Some(upload.media_key.to_vec());
            $media.file_sha256 = Some(upload.file_sha256.to_vec());
            $media.file_enc_sha256 = Some(upload.file_enc_sha256.to_vec());
            $media.file_length = Some(upload.file_length);
            $media.media_key_timestamp = Some(upload.media_key_timestamp);
            $media.mimetype = Some(file.mime_type.clone());
            if let Some(context) = context {
                $media.context_info = context.into();
            }
        }};
    }
    match file.kind {
        FileKind::Image => {
            let mut image = wa::message::ImageMessage::default();
            fill!(image);
            image.caption = caption;
            message.image_message = image.into();
        }
        FileKind::Video => {
            let mut video = wa::message::VideoMessage::default();
            fill!(video);
            video.caption = caption;
            video.streaming_sidecar = upload.streaming_sidecar;
            message.video_message = video.into();
        }
        FileKind::Audio => {
            let mut audio = wa::message::AudioMessage::default();
            let sidecar = upload.streaming_sidecar.clone();
            fill!(audio);
            audio.streaming_sidecar = sidecar;
            message.audio_message = audio.into();
        }
        FileKind::Document => {
            let mut document = wa::message::DocumentMessage::default();
            fill!(document);
            document.file_name.clone_from(&file.file_name);
            document.title.clone_from(&file.file_name);
            document.caption = caption;
            message.document_message = document.into();
        }
    }
    message
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_kept_reference_downloads_as_the_kind_it_came_as() {
        let mut image = wa::message::ImageMessage::default();
        image.direct_path = Some("/v/t62/abc".to_owned());
        image.media_key = Some(vec![7; 32]);
        image.file_sha256 = Some(vec![1; 32]);
        image.file_enc_sha256 = Some(vec![2; 32]);
        image.file_length = Some(1234);
        image.mimetype = Some("image/jpeg".to_owned());
        let mut message = wa::Message::default();
        message.image_message = image.into();

        let reference = MediaRef::of(&message).unwrap();
        assert_eq!(reference.key(), Some([7; 32]));
        let stored: MediaRef =
            serde_json::from_value(serde_json::to_value(&reference).unwrap()).unwrap();
        assert_eq!(stored, reference);

        let downloadable = reference.downloadable(Kind::Image).unwrap();
        assert_eq!(downloadable.direct_path(), Some("/v/t62/abc"));
        assert_eq!(downloadable.media_key(), Some(&[7; 32][..]));
        assert_eq!(downloadable.file_length(), Some(1234));
        assert!(matches!(downloadable.app_info(), MediaType::Image));
        assert!(reference.downloadable(Kind::Text).is_err());
    }

    #[test]
    fn a_message_without_a_fetchable_file_has_no_reference() {
        let mut text = wa::Message::default();
        text.conversation = Some("hi".to_owned());
        assert_eq!(MediaRef::of(&text), None);
        let mut image = wa::message::ImageMessage::default();
        image.media_key = Some(vec![7; 32]);
        let mut message = wa::Message::default();
        message.image_message = image.into();
        // No path to fetch it from
        assert_eq!(MediaRef::of(&message), None);
    }

    #[test]
    fn a_files_type_decides_how_it_goes_out() {
        assert_eq!(FileKind::of("image/png"), FileKind::Image);
        assert_eq!(FileKind::of("image/webp"), FileKind::Document);
        assert_eq!(FileKind::of("video/mp4"), FileKind::Video);
        assert_eq!(FileKind::of("audio/ogg"), FileKind::Audio);
        assert_eq!(FileKind::of("application/pdf"), FileKind::Document);
    }
}

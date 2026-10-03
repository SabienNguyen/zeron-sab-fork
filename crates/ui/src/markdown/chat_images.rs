//! Which Markdown images a chat reply draws inline, and where each is read.
//!
//! An image source resolves exactly like a file link: against the linking
//! chat's ordered checkouts, the chat's own first. The file is then read by
//! the device that owns it, through that chat's workspace file context, so a
//! remote chat's images load like its files do.
//!
//! Web images are never fetched. A reply can be steered by whatever the agent
//! read, and loading a URL it names would disclose the reader's address and
//! anything encoded in that URL to a third party; they keep their text.
use crate::workspace_links::{FileLinkResolution, FileLinkRoot, first_root_owning};

/// Formats the workspace image reader serves.
const EXTENSIONS: [&str; 7] = ["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp"];

/// One image read: the chat whose device and file context serve it, and the
/// wire path there — workspace-relative inside that chat's checkout, absolute
/// for a host file outside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ImageRead {
    pub chat: String,
    pub path: String,
}

impl ImageRead {
    /// The media cache key. A path never holds a control character, so the
    /// separator cannot occur in either half.
    pub(crate) fn key(&self) -> String {
        format!("{}\n{}", self.chat, self.path)
    }

    pub(crate) fn from_key(key: &str) -> Option<Self> {
        let (chat, path) = key.split_once('\n')?;
        Some(Self {
            chat: chat.to_owned(),
            path: path.to_owned(),
        })
    }
}

/// Whether a resolved image file loads without the reader asking for it.
/// `outside` marks a host file under none of the chat's checkouts
/// (`/tmp/plot.png`, a screenshot in the home directory): the same read a
/// click on its link performs, here started by the reply alone.
fn admits(path: &str, outside: bool) -> bool {
    // Agents routinely write screenshots and plots to scratch directories,
    // and the file never leaves the reader's own devices.
    let _ = outside;
    path.rsplit_once('.')
        .is_some_and(|(_, extension)| EXTENSIONS.iter().any(|e| extension.eq_ignore_ascii_case(e)))
}

/// The read an image `source` in a reply of `chat` stands for; `None` keeps
/// the image's text.
pub(crate) fn resolve(source: &str, chat: &str, roots: &[FileLinkRoot]) -> Option<ImageRead> {
    let (chat, path, outside) =
        match first_root_owning(source, roots.iter().map(|root| root.root.as_str()))? {
            FileLinkResolution::Owned { root, link } => {
                let root = &roots[root];
                match &root.chat {
                    Some(owner) => (owner.as_str(), link.path, false),
                    // A bare project root has no file context of its own: the
                    // linking chat reads it by absolute path.
                    None => (chat, root.absolute(&link).to_str()?.to_owned(), false),
                }
            }
            FileLinkResolution::Outside(link) => (chat, link.path, true),
        };
    admits(&path, outside).then(|| ImageRead {
        chat: chat.to_owned(),
        path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Vec<FileLinkRoot> {
        vec![
            FileLinkRoot {
                chat: Some("chat".into()),
                root: "/work/app".into(),
                local: true,
            },
            FileLinkRoot {
                chat: Some("parent".into()),
                root: "/work/parent".into(),
                local: true,
            },
            FileLinkRoot {
                chat: None,
                root: "/work/other".into(),
                local: true,
            },
        ]
    }

    fn read(chat: &str, path: &str) -> Option<ImageRead> {
        Some(ImageRead {
            chat: chat.into(),
            path: path.into(),
        })
    }

    #[test]
    fn sources_resolve_like_file_links_against_the_chat_roots() {
        let roots = roots();
        let resolve = |source| resolve(source, "chat", &roots);
        assert_eq!(resolve("shots/a.png"), read("chat", "shots/a.png"));
        assert_eq!(
            resolve("/work/app/shots/a.PNG"),
            read("chat", "shots/a.PNG")
        );
        assert_eq!(resolve("/work/parent/b.svg"), read("parent", "b.svg"));
        assert_eq!(
            resolve("/work/other/c.webp"),
            read("chat", "/work/other/c.webp")
        );
        assert_eq!(resolve("/tmp/plot.png"), read("chat", "/tmp/plot.png"));
        assert_eq!(
            resolve("file:///tmp/my%20plot.png"),
            read("chat", "/tmp/my plot.png")
        );
    }

    #[test]
    fn web_images_escapes_and_other_files_keep_their_text() {
        let roots = roots();
        for source in [
            "https://example.com/a.png",
            "http://example.com/a.png",
            "data:image/png;base64,AAAA",
            "../outside.png",
            "notes.md",
            "/tmp/archive.tar",
            "",
        ] {
            assert_eq!(resolve(source, "chat", &roots), None, "{source}");
        }
        assert_eq!(resolve("a.png", "chat", &[]), None);
    }

    #[test]
    fn keys_round_trip() {
        let read = read("chat", "/tmp/my plot.png").unwrap();
        assert_eq!(ImageRead::from_key(&read.key()), Some(read));
    }
}

//! Custom deserialization for the RXH configuration file.

use std::net::SocketAddr;

use serde::{
    de::{self, Visitor},
    Deserialize,
    Deserializer,
    Serialize,
};

use super::{Action, Pattern, Server};

/// See [`one_or_many`] for details.
#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum OneOrMany<T> {
    One(T),
    Many(Vec<T>),
}

impl<T> Into<Vec<T>> for OneOrMany<T> {
    fn into(self) -> Vec<T> {
        match self {
            OneOrMany::One(item) => vec![item],
            OneOrMany::Many(items) => items,
        }
    }
}

/// Helper for deserializing any type `T` into [`Vec<T>`]. This is useful for
/// configurations that allow omitting the array syntax. For example this TOML:
///
/// ```toml
/// [[server]]
///
/// listen = "127.0.0.1:8100"
/// ```
///
/// Should be deserialized as if an array was written instead:
///
/// ```toml
/// [[server]]
///
/// listen = ["127.0.0.1:8100"]
/// ```
pub(super) fn one_or_many<'de, T, D>(deserializer: D) -> Result<Vec<T>, D::Error>
where
    T: Deserialize<'de>,
    D: Deserializer<'de>,
{
    Ok(OneOrMany::deserialize(deserializer)?.into())
}

/// Implements [`Visitor`] to provide us with a custom deserialization of the
/// [`Server`] struct.
struct ServerVisitor;

/// Possible fields of a server instance in the config file.
#[derive(Deserialize)]
#[serde(field_identifier, rename_all = "lowercase")]
enum Field {
    Listen,
    Match,
    Forward,
    Serve,
    Uri,
}

/// Custom errors that can happen while manually deserializing [`Server`].
#[derive(Debug)]
enum Error {
    /// The config file allows either a `match` key assigned to an array of
    /// patterns or a simple untagged pattern:
    ///
    /// ```toml
    /// # Simple pattern example.
    ///
    /// [[server]]
    ///
    /// listen = "127.0.0.1:8000"
    /// forward = "127.0.0.1:9000"
    /// uri = "/api"
    ///
    /// # Match clause example.
    ///
    /// [[server]]
    ///
    /// listen = "128.0.01:8001"
    ///
    /// match = [
    ///     { uri = "/front", serve = "/home/website" },
    ///     { uri = "/brack", forward = "127.0.0.1:9001" },
    /// ]
    /// ```
    ///
    /// Both at the same time are not allowed. This is incorrect:
    ///
    /// ```toml
    /// [[server]]
    ///
    /// listen = "127.0.0.1:8000"
    /// forward = "127.0.0.1:9000"
    /// uri = "/api"
    /// match = [
    ///     { uri = "/front", serve = "/home/website" },
    ///     { uri = "/brack", forward = "127.0.0.1:9001" },
    /// ]
    /// ```
    MixedSimpleAndMatch,

    /// Simple patterns can't mix different actions. This is incorrect:
    ///
    /// ```toml
    /// [[server]]
    ///
    /// listen = "127.0.0.1:8000"
    /// forward = "127.0.0.1:9000"
    /// server = "/home/user/website"
    /// ```
    MixedActions,

    /// Couldn't find `match` clause or simple pattern.
    MissingConfig,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::MixedSimpleAndMatch => {
                f.write_str("either use 'match' for multiple patterns or describe a single pattern")
            }
            Error::MixedActions => f.write_str(
                "use either 'forward' or 'serve', if you need multiple patterns use 'match'",
            ),
            Error::MissingConfig => f.write_str("missing 'match' or simple configuration"),
        }
    }
}

impl<'de> Deserialize<'de> for Server {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_struct("Server", &["listen", "patterns"], ServerVisitor)
    }
}

impl<'de> Visitor<'de> for ServerVisitor {
    type Value = Server;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("at least 'listen' and 'forward' or 'serve' fields")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: de::MapAccess<'de>,
    {
        let mut listen: Vec<SocketAddr> = vec![];
        let mut patterns: Vec<Pattern> = vec![];
        let mut simple_pattern: Option<Pattern> = None;
        let mut uri = super::default::uri();

        while let Some(key) = map.next_key()? {
            match key {
                Field::Listen => {
                    if !listen.is_empty() {
                        return Err(de::Error::duplicate_field("listen"));
                    }

                    listen = map.next_value::<OneOrMany<SocketAddr>>()?.into();
                }

                Field::Match => {
                    if !patterns.is_empty() {
                        return Err(de::Error::duplicate_field("listen"));
                    }

                    if simple_pattern.is_some() {
                        return Err(de::Error::custom(Error::MixedSimpleAndMatch));
                    }

                    patterns = map.next_value()?;
                }

                Field::Forward => {
                    if !patterns.is_empty() {
                        return Err(de::Error::custom(Error::MixedSimpleAndMatch));
                    }

                    if let Some(pattern) = simple_pattern {
                        return match pattern.action {
                            Action::Forward(_) => Err(de::Error::duplicate_field("forward")),
                            Action::Serve(_) => Err(de::Error::custom(Error::MixedActions)),
                        };
                    }

                    simple_pattern = Some(Pattern {
                        uri: super::default::uri(),
                        action: Action::Forward(map.next_value::<OneOrMany<SocketAddr>>()?.into()),
                    });
                }

                Field::Serve => {
                    if !patterns.is_empty() {
                        return Err(de::Error::custom(Error::MixedSimpleAndMatch));
                    }

                    if let Some(pattern) = simple_pattern {
                        return match pattern.action {
                            Action::Forward(_) => Err(de::Error::custom(Error::MixedActions)),
                            Action::Serve(_) => Err(de::Error::duplicate_field("serve")),
                        };
                    }

                    simple_pattern = Some(Pattern {
                        uri: super::default::uri(),
                        action: Action::Serve(map.next_value()?),
                    });
                }

                Field::Uri => {
                    if !patterns.is_empty() {
                        return Err(de::Error::custom(Error::MixedSimpleAndMatch));
                    }

                    uri = map.next_value()?;
                }
            }
        }

        if let Some(mut pattern) = simple_pattern {
            pattern.uri = uri;
            patterns.push(pattern);
        }

        if patterns.is_empty() {
            return Err(de::Error::custom(Error::MissingConfig));
        }

        if listen.is_empty() {
            return Err(de::Error::missing_field("listen"));
        }

        Ok(Server { listen, patterns })
    }
}
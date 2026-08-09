use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use ts_rs::TS;

macro_rules! string_id {
    ($name:ident) => {
        #[derive(
            Clone,
            Debug,
            PartialEq,
            Eq,
            PartialOrd,
            Ord,
            Hash,
            Serialize,
            Deserialize,
            JsonSchema,
            TS,
        )]
        pub struct $name(pub String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<&str> for $name {
            fn from(value: &str) -> Self {
                Self::new(value)
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self::new(value)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

string_id!(EventId);
string_id!(ProjectId);
string_id!(ThreadId);
string_id!(RunId);
string_id!(OrchestrationId);
string_id!(WorkerId);
string_id!(CorrelationId);
string_id!(ArtifactId);
string_id!(CheckpointId);
string_id!(AgentSessionId);

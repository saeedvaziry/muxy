pub mod client;
pub mod extension;
pub mod framing;
pub mod hook;
pub mod server;

pub use extension::{
    EXTENSION_BROADCAST_HEAD, EXTENSION_LOCAL_EVENT_HEAD, EXTENSION_LOCAL_EVENT_MAX_NAME_LENGTH,
    EXTENSION_LOCAL_EVENT_MAX_PAYLOAD_BYTES, EXTENSION_LOCAL_EVENT_NAME_PREFIX, ExtensionBroadcast,
    ExtensionLocalEvent, INVOKE_HEAD, INVOKE_RESULT_HEAD, InvokeOutcome, InvokeRequest,
    InvokeResult, MODAL_QUERY_HEAD, MODAL_RESULT_HEAD, ModalQuery, ModalResult,
    is_valid_extension_local_event_name,
};
pub use framing::{
    CLI_REPLY_TERMINATOR, DEFAULT_MAX_INPUT_BYTES, InputAccumulator, InputError, InputRecord,
    LINE_TERMINATOR, MAX_READ_BYTES, frame_cli_reply, frame_persistent_line,
};
pub use hook::{
    AGENT_HOOK_ACKNOWLEDGEMENT_KIND, AGENT_HOOK_DEDUP_CAPACITY, AGENT_HOOK_EVENT_KIND,
    AGENT_HOOK_VERSION, AgentHookAcknowledgement, AgentHookEvent, AgentHookParseError,
    AgentHookPhase, RecentAgentHookEventIds, encode_agent_hook_acknowledgement,
    encode_agent_hook_event, parse_agent_hook_acknowledgement, parse_agent_hook_event,
};

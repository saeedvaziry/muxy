mod consent;
mod dispatcher;
mod policy;
mod services;
mod task;
mod transport;

pub use consent::{
    ConsentChoice, ConsentCoordinator, ConsentGate, ConsentRequest, ConsentResolution,
    ExtensionGatedPayload, GrantEvaluation, MAX_QUEUED_PROMPTS_PER_EXTENSION, PROMPT_TIMEOUT,
    apply_consent_choice, audit_entry, consent_request, consent_request_with_host_lookup,
    evaluate_grants, gated_verb_name, now_timestamp,
};
pub use dispatcher::{
    DispatchOutcome, ExtensionApiDispatcher, ExtensionApiService, ExtensionServiceDispatch,
    UnavailableExtensionApiService,
};
pub use policy::{ExtensionApiError, ensure_method_available, preflight_request};
pub use services::{ExtensionApiServiceGroup, extension_api_service_group};
pub use task::{ExtensionTaskContext, dispatch_extension_task, is_mutating_extension_task};
pub use transport::{
    DecodedSocketRequest, ExtensionApiRequest, ExtensionApiSource, ExtensionSocketReplyKind,
    MAX_SOCKET_PAYLOAD_BYTES, MAX_WEBVIEW_MESSAGE_BYTES, decode_extension_socket_request,
    decode_socket_request, decode_webview_request, encode_extension_socket_result,
    encode_socket_result, encode_webview_result, invalid_webview_message,
};

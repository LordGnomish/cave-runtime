//! Real behavior tests for cave-trace propagation (Mode B-prime spike).
//! Exercises W3C Trace Context parse/inject round-trip + error cases.
//! Generated 2026-05-04 via local Ollama (qwen3.6:35b-a3b-coding-mxfp8).
#![allow(unused_imports, unused_variables, unused_mut, dead_code)]

#[cfg(test)]
mod tests {
    use cave_trace::propagation::{
        parse_traceparent,
        parse_tracestate,
        extract_or_new,
        inject,
        VERSION,
        FLAG_SAMPLED,
        TRACEPARENT,
        TRACESTATE,
        PropagationError,
    };

    // Test a: Valid sampled traceparent parsing
    #[test]
    fn parse_traceparent_valid_sampled() {
        let header = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331-01";
        let result = parse_traceparent(header);
        
        assert!(result.is_ok(), "Expected valid traceparent, got error: {:?}", result.err());
        
        let tp = result.unwrap();
        
        assert_eq!(tp.version, VERSION);
        assert_eq!(tp.version, 0x00);
        assert_eq!(tp.flags, FLAG_SAMPLED);
        assert_eq!(tp.flags, 0x01);
        
        // Verify trace_id
        assert_eq!(tp.trace_id, 0x0af7651916cd43dd8448eb211c80319c);
        assert_ne!(tp.trace_id, 0);
        
        // Verify span_id
        assert_eq!(tp.span_id, 0xb7ad6b7169203331);
        assert_ne!(tp.span_id, 0);
    }

    // Test b: Wrong field count results in WrongFieldCount error
    #[test]
    fn parse_traceparent_wrong_field_count_errors() {
        // Missing the flags field (only 3 parts instead of 4)
        let header = "00-0af7651916cd43dd8448eb211c80319c-b7ad6b7169203331";
        let result = parse_traceparent(header);
        
        assert!(result.is_err(), "Expected error for malformed traceparent");
        assert!(matches!(result.unwrap_err(), PropagationError::WrongFieldCount));
    }

    // Test c: Inject and parse round-trip preserves data
    #[test]
    fn inject_parse_round_trip() {
        let original_tp = cave_trace::propagation::TraceParent {
            version: VERSION,
            trace_id: 0x0af7651916cd43dd8448eb211c80319c,
            span_id: 0xb7ad6b7169203331,
            flags: FLAG_SAMPLED,
        };
        
        let original_ts = parse_tracestate("");
        
        let (injected_tp_str, injected_ts_str) = inject(&original_tp, &original_ts);
        
        // Verify the injected string format is correct (basic sanity)
        assert!(!injected_tp_str.is_empty());
        assert!(injected_tp_str.starts_with("00-"));
        
        // Parse it back
        let parsed_tp = parse_traceparent(&injected_tp_str).expect("Round-trip injection should yield valid traceparent");
        
        // Assert fields match
        assert_eq!(parsed_tp.version, original_tp.version);
        assert_eq!(parsed_tp.trace_id, original_tp.trace_id);
        assert_eq!(parsed_tp.span_id, original_tp.span_id);
        assert_eq!(parsed_tp.flags, original_tp.flags);
    }

    // Test d: extract_or_new with None returns fresh context with non-zero IDs
    #[test]
    fn extract_or_new_with_none_returns_fresh() {
        let (tp, ts) = extract_or_new(None, None);
        
        // The implementation should generate a new trace context
        // TraceId and SpanId must be non-zero to be valid
        assert_ne!(tp.trace_id, 0, "Generated trace_id should not be zero");
        assert_ne!(tp.span_id, 0, "Generated span_id should not be zero");
        
        // Version should be the current spec version
        assert_eq!(tp.version, VERSION);
    }

    // Test e: Constants match W3C Trace Context spec expectations
    #[test]
    fn parse_tracestate_constants_match_w3c() {
        // Verify the public constants match the W3C Trace Context specification
        assert_eq!(TRACEPARENT, "traceparent");
        assert_eq!(TRACESTATE, "tracestate");
        assert_eq!(VERSION, 0x00);
        assert_eq!(FLAG_SAMPLED, 0x01);
        
        // Additional sanity check on max limits
        assert_eq!(cave_trace::propagation::MAX_TRACESTATE_ENTRIES, 32);
        assert_eq!(cave_trace::propagation::MAX_TRACESTATE_LEN, 512);
    }
}

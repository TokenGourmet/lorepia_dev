macro_rules! with_lorepia_app_commands {
    ($consumer:ident) => {
        $consumer! {
            ack_stream,
            cancel_stream,
            get_stream_snapshot,
            host_broker_probe_count,
            host_broker_request,
            register_host_broker_session,
            rotate_host_broker_session,
            start_mock_stream,
        }
    };
}

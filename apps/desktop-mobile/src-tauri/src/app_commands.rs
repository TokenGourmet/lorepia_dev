macro_rules! with_product_app_commands {
    ($macro:ident) => {
        $macro!(
            get_product_bootstrap,
            get_provider_credential_status,
            save_provider_api_key,
            delete_provider_credential,
            start_provider_stream,
            ack_provider_stream,
            cancel_provider_stream,
            get_provider_stream_snapshot,
            get_storage_status,
            create_chat,
            list_chats,
            load_chat_messages,
            delete_chat,
            get_app_preferences,
            update_app_preferences,
        )
    };
}

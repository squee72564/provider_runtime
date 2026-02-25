use super::translator_contract::ProviderTranslator;

#[test]
fn test_internal_translator_contract_module_is_accessible_within_crate() {
    struct NoopTranslator;

    impl ProviderTranslator for NoopTranslator {
        type RequestPayload = ();
        type ResponsePayload = ();

        fn encode_request(
            &self,
            _req: &crate::core::types::ProviderRequest,
        ) -> Result<Self::RequestPayload, crate::core::error::ProviderError> {
            Ok(())
        }

        fn decode_response(
            &self,
            _payload: &Self::ResponsePayload,
        ) -> Result<crate::core::types::ProviderResponse, crate::core::error::ProviderError>
        {
            Err(crate::core::error::ProviderError::Protocol {
                provider: crate::core::types::ProviderId::Other("Other".to_string()),
                model: None,
                request_id: None,
                message: "not implemented".to_string(),
            })
        }
    }

    let _translator: &dyn ProviderTranslator<RequestPayload = (), ResponsePayload = ()> =
        &NoopTranslator;
}

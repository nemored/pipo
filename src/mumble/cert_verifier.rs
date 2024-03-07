use std::time::SystemTime;

use rustls::{
    client::{ServerCertVerified, ServerCertVerifier},
    Certificate, Error, ServerName,
};

pub struct MumbleCertVerifier {
    cert: Certificate,
}

impl MumbleCertVerifier {
    pub fn new(cert: Certificate) -> Self {
        Self { cert }
    }
}

impl ServerCertVerifier for MumbleCertVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &Certificate,
        intermediates: &[Certificate],
        server_name: &ServerName,
        scts: &mut dyn Iterator<Item = &[u8]>,
        ocsp_response: &[u8],
        now: SystemTime,
    ) -> Result<ServerCertVerified, Error> {
        if *end_entity == self.cert {
            Ok(ServerCertVerified::assertion())
        } else {
            let error = String::from(
                "invalid peer certificate: does not match certificate loaded from server_cert file",
            );

            // TODO: maybe this shouldn't be a General error
            Err(Error::General(error))
        }
    }
}

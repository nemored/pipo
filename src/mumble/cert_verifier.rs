use std::{time::SystemTime, sync::Arc, fmt::{Display, Formatter, self}};

use rustls::{client::{ServerCertVerifier, ServerCertVerified}, ServerName, Certificate, CertificateError, Error};

#[derive(Debug)]
struct DoesNotMatch;

impl Display for DoesNotMatch {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "invalid peer certificate: does not match certificate loaded from server_cert file")
    }
}

impl std::error::Error for DoesNotMatch {}

pub struct MumbleCertVerifier {
    cert: Certificate
}

impl MumbleCertVerifier {
    pub fn new(cert: Certificate) -> Self {
        Self { cert }
    }
}

impl ServerCertVerifier for MumbleCertVerifier {
    fn verify_server_cert(&self,
                          end_entity: &Certificate,
                          intermediates: &[Certificate],
                          server_name: &ServerName,
                          scts: &mut dyn Iterator<Item = &[u8]>,
                          ocsp_response: &[u8],
                          now: SystemTime)
                          -> Result<ServerCertVerified, Error> {
        if *end_entity == self.cert {
            Ok(ServerCertVerified::assertion())
        }
        else {
            let error = Arc::new(DoesNotMatch);
            Err(Error::InvalidCertificate(CertificateError::Other(error)))
        }
    }
}

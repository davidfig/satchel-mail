//! Click-tracking redirectors that newsletter and marketing services wrap
//! every link in. A link through one hides where it goes instead of naming
//! the wrong place, so `deceptive_links` doesn't judge it: otherwise nearly
//! every newsletter would read as a scam.

/// Hosts used only for redirecting tracked clicks, matched as the whole host
/// or a suffix after a dot. Kept to hosts that serve nothing else, so a page
/// someone publishes on a shared host (a Beehiiv newsletter, a cloud bucket)
/// still counts.
const TRACKER_HOSTS: &[&str] = &[
    "ct.sendgrid.net", // SendGrid
    "list-manage.com", // Mailchimp
    "mailchi.mp", // Mailchimp
    "link.mail.beehiiv.com", // Beehiiv
    "click.revue.email", // Revue
    "govdelivery.com", // Granicus GovDelivery
    "adobe-campaign.com", // Adobe Campaign
    "track.email.us-phoenix-1.oci.oraclecloud.com", // Oracle Email Delivery
    "rs6.net", // Constant Contact
    "hubspotlinks.com", // HubSpot
    "klclick.com", // Klaviyo
    "klclick1.com", // Klaviyo
    "klclick2.com", // Klaviyo
    "klclick3.com", // Klaviyo
    "awstrack.me", // Amazon SES
    "click.pstmrk.it", // Postmark
    "mjt.lu", // Mailjet
    "spgo.io", // SparkPost
    "links.iterable.com", // Iterable
];

/// Whether `host` is a known click-tracking redirector.
pub(crate) fn is_click_tracker(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    TRACKER_HOSTS.iter().any(|t| host == *t || host.strip_suffix(t).is_some_and(|rest| rest.ends_with('.')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trackers() {
        assert!(is_click_tracker("u46392770.ct.sendgrid.net"));
        assert!(is_click_tracker("memberbenefits.us4.list-manage.com"));
        assert!(is_click_tracker("links-1.govdelivery.com"));
        assert!(is_click_tracker("cell0.track.email.us-phoenix-1.oci.oraclecloud.com"));
        // Look-alikes and other pages on a shared host don't count.
        assert!(!is_click_tracker("evil-list-manage.com"));
        assert!(!is_click_tracker("list-manage.com.evil.test"));
        assert!(!is_click_tracker("scam.beehiiv.com"));
        assert!(!is_click_tracker("bucket.oraclecloud.com"));
    }
}

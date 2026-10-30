//! Click-tracking redirectors that newsletter and marketing services wrap
//! every link in. A link through one hides where it goes instead of naming
//! the wrong place, so `deceptive_links` doesn't judge it: otherwise nearly
//! every newsletter would read as a scam.

/// Redirectors as (host, path prefix): the host matches whole or as a suffix
/// after a dot. Hosts that serve nothing but redirects have an empty prefix.
/// Where a host also serves pages anyone can make (Mailchimp's signup forms,
/// GovDelivery's bulletins), only the redirect path counts, and hosts made
/// for publishing (mailchi.mp landing pages, a Beehiiv newsletter, a cloud
/// bucket) aren't listed at all.
const TRACKERS: &[(&str, &str)] = &[
    ("ct.sendgrid.net", ""), // SendGrid
    ("list-manage.com", "/track/click"), // Mailchimp
    ("link.mail.beehiiv.com", ""), // Beehiiv
    ("click.revue.email", ""), // Revue
    ("govdelivery.com", "/CL0/"), // Granicus GovDelivery
    ("govdelivery.com", "/track"), // Granicus GovDelivery, older links
    ("adobe-campaign.com", ""), // Adobe Campaign
    ("track.email.us-phoenix-1.oci.oraclecloud.com", ""), // Oracle Email Delivery
    ("rs6.net", ""), // Constant Contact
    ("hubspotlinks.com", ""), // HubSpot
    ("klclick.com", ""), // Klaviyo
    ("klclick1.com", ""), // Klaviyo
    ("klclick2.com", ""), // Klaviyo
    ("klclick3.com", ""), // Klaviyo
    ("awstrack.me", ""), // Amazon SES
    ("click.pstmrk.it", ""), // Postmark
    ("mjt.lu", ""), // Mailjet
    ("spgo.io", ""), // SparkPost
    ("links.iterable.com", ""), // Iterable
];

/// Whether a link to `host` with `path` goes through a known click-tracking redirector.
pub(crate) fn is_click_tracker(host: &str, path: &str) -> bool {
    let host = host.trim_end_matches('.');
    TRACKERS.iter().any(|(t, prefix)| (host == *t || host.strip_suffix(t).is_some_and(|rest| rest.ends_with('.'))) && path.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trackers() {
        assert!(is_click_tracker("u46392770.ct.sendgrid.net", "/ls/click"));
        assert!(is_click_tracker("memberbenefits.us4.list-manage.com", "/track/click"));
        assert!(is_click_tracker("links-1.govdelivery.com", "/CL0/https:%2F%2Fexample.gov/1/0100"));
        assert!(is_click_tracker("cell0.track.email.us-phoenix-1.oci.oraclecloud.com", "/"));
        // Look-alikes and other pages on a shared host don't count.
        assert!(!is_click_tracker("evil-list-manage.com", "/track/click"));
        assert!(!is_click_tracker("list-manage.com.evil.test", "/track/click"));
        assert!(!is_click_tracker("x.us4.list-manage.com", "/subscribe"));
        assert!(!is_click_tracker("content.govdelivery.com", "/accounts/X/bulletins/1"));
        assert!(!is_click_tracker("mailchi.mp", "/x/login"));
        assert!(!is_click_tracker("scam.beehiiv.com", "/"));
        assert!(!is_click_tracker("bucket.oraclecloud.com", "/"));
    }
}

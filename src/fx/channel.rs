use super::Biquad;
pub(super) struct Channel {
    pub(super) eq: [Biquad; 7],
    pub(super) loud: [Biquad; 2],
}

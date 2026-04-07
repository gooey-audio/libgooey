#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Interval {
    Unison,
    MinorSecond,
    MajorSecond,
    MinorThird,
    MajorThird,
    PerfectFourth,
    Tritone,
    PerfectFifth,
    MinorSixth,
    MajorSixth,
    MinorSeventh,
    MajorSeventh,
    Octave,
    MinorNinth,
    MajorNinth,
    MinorTenth,
    MajorTenth,
    PerfectEleventh,
    SharpEleventh,
    PerfectTwelfth,
    MinorThirteenth,
    MajorThirteenth,
}

impl Interval {
    pub fn semitones(self) -> u8 {
        match self {
            Interval::Unison => 0,
            Interval::MinorSecond => 1,
            Interval::MajorSecond => 2,
            Interval::MinorThird => 3,
            Interval::MajorThird => 4,
            Interval::PerfectFourth => 5,
            Interval::Tritone => 6,
            Interval::PerfectFifth => 7,
            Interval::MinorSixth => 8,
            Interval::MajorSixth => 9,
            Interval::MinorSeventh => 10,
            Interval::MajorSeventh => 11,
            Interval::Octave => 12,
            Interval::MinorNinth => 13,
            Interval::MajorNinth => 14,
            Interval::MinorTenth => 15,
            Interval::MajorTenth => 16,
            Interval::PerfectEleventh => 17,
            Interval::SharpEleventh => 18,
            Interval::PerfectTwelfth => 19,
            Interval::MinorThirteenth => 20,
            Interval::MajorThirteenth => 21,
        }
    }
}

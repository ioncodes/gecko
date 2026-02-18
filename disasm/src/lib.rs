pub mod tokenizer;

pub mod gekko {
    include!(concat!(env!("OUT_DIR"), "/gekko.rs"));
}

pub mod dsp {
    include!(concat!(env!("OUT_DIR"), "/dsp.rs"));
}

pub mod cpu {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Gpr(pub u8);

    impl std::fmt::Display for Gpr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "r{}", self.0)
        }
    }

    impl From<u8> for Gpr {
        fn from(val: u8) -> Self {
            Gpr(val)
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct Fpr(pub u8);

    impl std::fmt::Display for Fpr {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "f{}", self.0)
        }
    }

    impl From<u8> for Fpr {
        fn from(val: u8) -> Self {
            Fpr(val)
        }
    }
}
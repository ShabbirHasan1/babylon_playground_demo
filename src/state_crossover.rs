use bitflags::bitflags;

bitflags! {
    // The following abbreviations stand for:
    // Fast, Medium, Slow, High, Low, Highest-High, Lowest-Low, Price
    // O(ver), U(nder)
    #[derive(Default)]
    pub struct C: u64 {
        // moving averages
        const FOM = 1 << 0;
        const FOS = 1 << 1;
        const MOF = 1 << 2;
        const MOS = 1 << 3;
        const SOF = 1 << 4;
        const SOM = 1 << 5;
        const FUM = 1 << 6;
        const FUS = 1 << 7;
        const MUF = 1 << 8;
        const MUS = 1 << 9;
        const SUF = 1 << 10;
        const SUM = 1 << 11;
        const POF = 1 << 12;
        const POM = 1 << 13;
        const POS = 1 << 14;
        const PUF = 1 << 15;
        const PUM = 1 << 16;
        const PUS = 1 << 17;
        const FOP = 1 << 18;
        const MOP = 1 << 19;
        const SOP = 1 << 20;
        const FUP = 1 << 21;
        const MUP = 1 << 22;
        const SUP = 1 << 23;
        // bounds
        const FOH = 1 << 24;
        const FUH = 1 << 25;
        const HOF = 1 << 26;
        const HUF = 1 << 27;
        const FOL = 1 << 28;
        const FUL = 1 << 29;
        const LOF = 1 << 30;
        const LUF = 1 << 31;
        const POHH = 1 << 32;
        const PUHH = 1 << 33;
        const HHOP = 1 << 34;
        const HHUP = 1 << 35;
        const POLL = 1 << 36;
        const PULL = 1 << 37;
        const LLOP = 1 << 38;
        const LLUP = 1 << 39;
        const POH = 1 << 40;
        const PUH = 1 << 41;
        const HOP = 1 << 42;
        const HUP = 1 << 43;
        const POL = 1 << 44;
        const PUL = 1 << 45;
        const LOP = 1 << 46;
        const LUP = 1 << 47;
        // high & low intersections
        const SOL = 1 << 48;
        const SUL = 1 << 49;
        const LOS = 1 << 50;
        const LUS = 1 << 51;
        const HOL = 1 << 52;
        const HUL = 1 << 53;
        const LOH = 1 << 54;
        const LUH = 1 << 55;
        const SUH = 1 << 56;
        const SOH = 1 << 57;
        const HOS = 1 << 58;
        const HUS = 1 << 59;
    }
}

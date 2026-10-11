class A {
    long m(int days, int x) {
        long millis = days * 24 * 3600 * 1000; // expect
        long ok = days * 24L * 3600 * 1000;
        long small = x * 2;
        long big = x * x; // expect
        return days * 86_400_000; // expect
    }
}
class Varint {
    long decode(int b1, int b) {
        long r = b1 & 0x7F;
        r |= (b & 0x7F) << 14;
        return r;
    }
    long product(byte a, byte b) { long p = a * b; return p; }
}
class Wrapped {
    long constant() { return 65536 * 65536; } // expect
}
class SignedBits {
    long mask(int x) { return (x & -1) * 1000; } // expect
    long shift(byte b) { return (b >>> 1) * 1000; } // expect
}

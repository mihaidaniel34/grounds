class A {
    double m(int a, int b, long c) {
        double x = Math.ceil(a / b); // expect
        long y = Math.round(c); // expect
        double z = Math.ceil((double) a / b);
        return Math.floor((double) (a / b)); // expect
    }
}
class CastWithoutDivision {
    long backoff(long wait, int a, int b, int c) {
        long t = Math.round((double) wait);
        double s = Math.ceil((double) (a * b + c));
        return Math.round((double) (a + b / c)); // expect
    }
}
class HiddenDivision {
    double m(int a, int b, long n, boolean f) {
        long r = Math.round((long) n); // expect
        double x = Math.floor((double) -(a / b)); // expect
        double y = Math.ceil((double) (long) (a / b)); // expect
        return Math.ceil((double) (f ? a / b : a)); // expect
    }
}
class DivisionUnderRemainderOrSwitch {
    double m(int a, int b, int c, int mode) {
        double r = Math.ceil((double) ((a / b) % c)); // expect
        return Math.ceil((double) switch (mode) { // expect
            case 1 -> a / b;
            default -> 0;
        });
    }
}

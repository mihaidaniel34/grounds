// a case label of a switch on an enum names the enum's constant (JLS
// 14.11.1): a field of the same name in scope is not read there
enum Phase { START, STOP }
class Machine {
    private static final Object START = new Object(); // expect
    private static final int STOP = 2;
    int step(Phase p) {
        switch (p) {
            case START: return 1;
            default: return 0;
        }
    }
    int next(Phase p) {
        return switch (p) { case START, STOP -> 1; };
    }
    // on an int selector the label is the constant itself
    int code(int i) {
        switch (i) {
            case STOP: return 1;
            default: return 0;
        }
    }
}

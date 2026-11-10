class A {
    int m(int x) {
        int r = 0;
        switch (x) {
            case 1:
                r = 1;
            case 2: // expect
                r = 2;
                break;
            case 3:
                r = 3;
                // fall through
            case 4:
                r = 4;
                break;
            case 5:
            case 6:
                return 6;
            default:
                r = 9;
        }
        switch (x) {
            case 1 -> r = 1;
            case 2 -> r = 2;
            default -> r = 0;
        }
        return r;
    }
}
class NestedSwitch {
    int m(int x, int op) {
        switch (x) {
            case 2:
                switch (op) { case 1: return 1; default: return 2; }
            default:
                return 0;
        }
    }
}
class NestedDefault {
    void m(int x, int y) {
        switch (x) {
            case 0:
                switch (y) { case 0: switch (x) { default: throw new RuntimeException(); } }
            case 1: // expect
                work();
                break;
        }
    }
    void work() {}
}
class ConditionalExit {
    String m(int kind, Object value, StringBuilder out) {
        switch (kind) {
            case 0:
                // leaves the switch when done, falls through otherwise: deliberate
                if (value == null) {
                    return null;
                }
            case 1:
                out.append(value);
                if (out.length() > 10) break;
            case 2:
                out.append('.');
                break;
            case 3:
                out.append("three");
            case 4: // expect
                out.append("four");
                break;
            case 5:
                for (int i = 0; i < 2; i++) { if (i == 1) break; }
            default: // expect
                out.append('?');
        }
        return out.toString();
    }
}

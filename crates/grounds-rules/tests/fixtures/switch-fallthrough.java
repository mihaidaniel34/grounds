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

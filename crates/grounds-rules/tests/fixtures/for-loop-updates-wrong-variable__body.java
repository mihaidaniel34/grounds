// grounds: no-findings
class A {
    int m() {
        int attempts = 0;
        for (int remaining = 3; remaining > 0; attempts++) {
            remaining--;
        }
        return attempts;
    }
}

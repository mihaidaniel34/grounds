// grounds: no-findings
class A {
    int m() {
        int count = 0;
        for (int low = 0, high = 10; low < high; low--) {
            high -= 2;
            count++;
        }
        return count;
    }
}

class A implements Comparable<A> {
    int v;
    public int compareTo(A o) { return Integer.compare(v, o.v); }
    int rev(A a, A b, String s, String t) {
        int r = -a.compareTo(b); // expect
        r = -s.compareTo(t); // expect
        return b.compareTo(a) + -r;
    }
}

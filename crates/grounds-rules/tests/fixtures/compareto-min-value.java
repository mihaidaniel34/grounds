class A implements Comparable<A> {
    int v;
    public int compareTo(A o) {
        if (o == null) return Integer.MIN_VALUE; // expect
        return Integer.compare(v, o.v);
    }
    int other() { return Integer.MIN_VALUE; }
}

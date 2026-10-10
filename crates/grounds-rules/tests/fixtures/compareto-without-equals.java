class A implements Comparable<A> {
    int v;
    public int compareTo(A o) { return Integer.compare(v, o.v); } // expect
}
class B implements Comparable<B> {
    int v;
    public int compareTo(B o) { return 0; }
    public boolean equals(Object o) { return o instanceof B; }
    public int hashCode() { return 0; }
}

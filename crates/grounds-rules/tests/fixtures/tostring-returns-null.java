class A implements Cloneable {
    String name;
    @Override public String toString() {
        if (name == null) return null; // expect
        return name;
    }
    @Override public Object clone() { return null; } // expect
    String other() { return null; }
}

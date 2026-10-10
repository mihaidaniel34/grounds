class A {
    private String name;
    private int count;
    A(String newName) {
        this.name = name; // expect
        count = count; // expect
    }
    void set(String name) {
        this.name = name;
        int x = 1;
        x = x; // expect
    }
}

class A {
    private final String name;
    private final int size;
    private final int fixed = 3;
    A(String n) {
        System.out.println(name); // expect
        this.name = n;
        int s = fixed;
        size = name.length();
    }
    A() { this("x"); }
}
class Outer {
    void run() {
        class Bank {
            final java.util.Map<String, Double> accounts = new java.util.HashMap<>();
            Bank() { accounts.put("Mario", 50.0); }
        }
    }
}

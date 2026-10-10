class A {
    void m() {
        try { Thread.sleep(10); } catch (InterruptedException e) { } // expect
        try { Thread.sleep(10); } catch (InterruptedException e) { System.out.println("x"); } // expect
        try { Thread.sleep(10); } catch (InterruptedException e) { Thread.currentThread().interrupt(); }
        try { Thread.sleep(10); } catch (InterruptedException e) { throw new RuntimeException(e); }
    }
}

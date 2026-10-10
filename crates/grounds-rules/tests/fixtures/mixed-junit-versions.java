// grounds: test
class ATest {
    @org.junit.Before public void setup() { } // expect
    @org.junit.jupiter.api.Test void t() { }
}
class BTest {
    @org.junit.jupiter.api.BeforeEach void setup() { }
    @org.junit.jupiter.api.Test void t() { }
}

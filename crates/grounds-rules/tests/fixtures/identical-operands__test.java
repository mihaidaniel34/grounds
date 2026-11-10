// grounds: test
class EqualsContractTest {
    void reflexive(Object key, int a) {
        // reflexivity is part of the equals contract
        boolean r = key.equals(key);
        if (a == a) return; // expect
    }
}

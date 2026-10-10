// grounds: no-findings
class Sample {
    volatile Object state = new Object();
    void publish() { state = state; }
}

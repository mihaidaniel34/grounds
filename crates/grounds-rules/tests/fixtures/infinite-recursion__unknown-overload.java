import missing.Exchange;
class Deliverer extends missing.BaseDeliverer {
    // the inner call may target the inherited findResource(List<String>)
    protected Object findResource(Exchange exchange) {
        return findResource(exchange.getRequest().getOptions().getUriPath());
    }
    int loop(int x) { return loop(x); } // expect
}

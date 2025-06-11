import { useState, useEffect } from "react";
import Button from "./button";
import { PaymentInfo, PaymentRequest, Route96 } from "../upload/admin";

interface PaymentFlowProps {
  route96: Route96;
  onPaymentRequested?: (paymentRequest: string) => void;
}

export default function PaymentFlow({ route96, onPaymentRequested }: PaymentFlowProps) {
  const [paymentInfo, setPaymentInfo] = useState<PaymentInfo | null>(null);
  const [gigabytes, setGigabytes] = useState<number>(1);
  const [months, setMonths] = useState<number>(1);
  const [paymentRequest, setPaymentRequest] = useState<string>("");
  const [error, setError] = useState<string>("");
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (paymentInfo === null) {
      loadPaymentInfo();
    }
  }, [paymentInfo]);

  async function loadPaymentInfo() {
    try {
      const info = await route96.getPaymentInfo();
      setPaymentInfo(info);
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message);
      } else {
        setError("Failed to load payment info");
      }
    }
  }

  async function requestPayment() {
    if (!paymentInfo) return;
    
    setLoading(true);
    setError("");
    
    try {
      const request: PaymentRequest = { units: gigabytes, quantity: months };
      const response = await route96.requestPayment(request);
      setPaymentRequest(response.pr);
      onPaymentRequested?.(response.pr);
    } catch (e) {
      if (e instanceof Error) {
        setError(e.message);
      } else {
        setError("Failed to request payment");
      }
    } finally {
      setLoading(false);
    }
  }

  if (error && !paymentInfo) {
    return <div className="text-red-400">Payment not available: {error}</div>;
  }

  if (!paymentInfo) {
    return <div className="text-gray-400">Loading payment info...</div>;
  }

  const totalCostBTC = paymentInfo.cost.amount * gigabytes * months;
  const totalCostSats = Math.round(totalCostBTC * 100000000); // Convert BTC to sats

  function formatStorageUnit(unit: string): string {
    if (unit.toLowerCase().includes('gbspace') || unit.toLowerCase().includes('gb')) {
      return 'GB';
    }
    return unit;
  }

  return (
    <div className="card">
      <h3 className="text-lg font-bold mb-4">Top Up Account</h3>
      
      <div className="space-y-4 mb-6">
        <div className="text-center">
          <div className="text-2xl font-bold text-gray-100 mb-2">
            {gigabytes} {formatStorageUnit(paymentInfo.unit)} for {months} month{months > 1 ? 's' : ''}
          </div>
          <div className="text-lg text-blue-400 font-semibold">
            {totalCostSats.toLocaleString()} sats
          </div>
        </div>
        
        <div className="grid grid-cols-2 gap-4">
          <div>
            <label className="block text-sm font-medium mb-2 text-gray-300">
              Storage ({formatStorageUnit(paymentInfo.unit)})
            </label>
            <input
              type="number"
              min="1"
              step="1"
              value={gigabytes}
              onChange={(e) => setGigabytes(parseInt(e.target.value) || 1)}
              className="input w-full text-center text-lg"
            />
          </div>
          
          <div>
            <label className="block text-sm font-medium mb-2 text-gray-300">
              Duration (months)
            </label>
            <input
              type="number"
              min="1"
              step="1"
              value={months}
              onChange={(e) => setMonths(parseInt(e.target.value) || 1)}
              className="input w-full text-center text-lg"
            />
          </div>
        </div>
      </div>


      <Button
        onClick={requestPayment}
        disabled={loading || gigabytes <= 0 || months <= 0}
        className="btn-primary w-full mb-4"
      >
        {loading ? "Processing..." : "Generate Payment Request"}
      </Button>

      {error && <div className="text-red-400 text-sm mb-4">{error}</div>}

      {paymentRequest && (
        <div className="bg-gray-800 p-4 rounded-lg border border-gray-700">
          <div className="text-sm font-medium mb-2">Lightning Invoice:</div>
          <div className="font-mono text-xs break-all bg-gray-900 p-2 rounded">
            {paymentRequest}
          </div>
          <div className="text-xs text-gray-400 mt-2">
            Copy this invoice to your Lightning wallet to complete payment
          </div>
        </div>
      )}
    </div>
  );
}
import { useState, useEffect } from "react";
import Button from "./button";
import { PaymentInfo, PaymentRequest, Route96 } from "../upload/admin";

interface PaymentFlowProps {
  route96: Route96;
  onPaymentRequested?: (paymentRequest: string) => void;
}

export default function PaymentFlow({ route96, onPaymentRequested }: PaymentFlowProps) {
  const [paymentInfo, setPaymentInfo] = useState<PaymentInfo | null>(null);
  const [units, setUnits] = useState<number>(1);
  const [quantity, setQuantity] = useState<number>(1);
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
      const request: PaymentRequest = { units, quantity };
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
    return <div className="text-red-500">Payment not available: {error}</div>;
  }

  if (!paymentInfo) {
    return <div>Loading payment info...</div>;
  }

  const totalCost = paymentInfo.cost.amount * units * quantity;

  return (
    <div className="bg-neutral-700 p-4 rounded-lg">
      <h3 className="text-lg font-bold mb-4">Top Up Account</h3>
      
      <div className="grid grid-cols-2 gap-4 mb-4">
        <div>
          <label className="block text-sm font-medium mb-1">
            Units ({paymentInfo.unit})
          </label>
          <input
            type="number"
            min="0.1"
            step="0.1"
            value={units}
            onChange={(e) => setUnits(parseFloat(e.target.value) || 0)}
            className="w-full px-3 py-2 bg-neutral-800 border border-neutral-600 rounded"
          />
        </div>
        
        <div>
          <label className="block text-sm font-medium mb-1">
            Quantity
          </label>
          <input
            type="number"
            min="1"
            value={quantity}
            onChange={(e) => setQuantity(parseInt(e.target.value) || 1)}
            className="w-full px-3 py-2 bg-neutral-800 border border-neutral-600 rounded"
          />
        </div>
      </div>

      <div className="mb-4">
        <div className="text-sm text-neutral-300">
          Cost: {totalCost.toFixed(8)} {paymentInfo.cost.currency} per {paymentInfo.interval}
        </div>
      </div>

      <Button
        onClick={requestPayment}
        disabled={loading || units <= 0 || quantity <= 0}
        className="w-full mb-4"
      >
        {loading ? "Processing..." : "Generate Payment Request"}
      </Button>

      {error && <div className="text-red-500 text-sm mb-4">{error}</div>}

      {paymentRequest && (
        <div className="bg-neutral-800 p-4 rounded">
          <div className="text-sm font-medium mb-2">Lightning Invoice:</div>
          <div className="font-mono text-xs break-all bg-neutral-900 p-2 rounded">
            {paymentRequest}
          </div>
          <div className="text-xs text-neutral-400 mt-2">
            Copy this invoice to your Lightning wallet to complete payment
          </div>
        </div>
      )}
    </div>
  );
}
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentPart {
    pub name: String,
    pub description: String,
    pub html: String,
    pub css: String,
    pub javascript: String,
    pub order: usize,
}

#[derive(Debug, Clone)]
pub struct IncrementalUiBuilder {
    prompt: String,
    parts: Arc<RwLock<Vec<ComponentPart>>>,
    current_part: Arc<RwLock<usize>>,
}

impl IncrementalUiBuilder {
    pub fn new(prompt: String) -> Self {
        Self {
            prompt,
            parts: Arc::new(RwLock::new(Vec::new())),
            current_part: Arc::new(RwLock::new(0)),
        }
    }

    pub async fn plan_components(&self) -> Result<Vec<String>> {
        let components = if self.prompt.to_lowercase().contains("bank")
            || self.prompt.to_lowercase().contains("portfolio")
        {
            vec![
                "Account Summary Header".to_string(),
                "Balance Cards".to_string(),
                "Transaction List".to_string(),
                "Stock Portfolio Table".to_string(),
                "Performance Chart".to_string(),
            ]
        } else {
            vec![
                "Header Section".to_string(),
                "Main Content".to_string(),
                "Footer Section".to_string(),
            ]
        };

        Ok(components)
    }

    pub async fn generate_next_part(&self) -> Result<Option<ComponentPart>> {
        let current = *self.current_part.read().await;
        let components = self.plan_components().await?;

        if current >= components.len() {
            return Ok(None);
        }

        let component_name = &components[current];
        let part = self.generate_component(component_name, current).await?;

        self.parts.write().await.push(part.clone());
        *self.current_part.write().await = current + 1;

        Ok(Some(part))
    }

    async fn generate_component(&self, name: &str, order: usize) -> Result<ComponentPart> {
        let (html, css, javascript) = match name {
            "Account Summary Header" => self.generate_header(),
            "Balance Cards" => self.generate_balance_cards(),
            "Transaction List" => self.generate_transaction_list(),
            "Stock Portfolio Table" => self.generate_portfolio_table(),
            "Performance Chart" => self.generate_chart(),
            "Header Section" => self.generate_generic_header(),
            "Main Content" => self.generate_generic_content(),
            "Footer Section" => self.generate_generic_footer(),
            _ => self.generate_generic_content(),
        };

        Ok(ComponentPart {
            name: name.to_string(),
            description: format!("Generated {name}"),
            html,
            css,
            javascript,
            order,
        })
    }

    fn generate_header(&self) -> (String, String, String) {
        let html = r#"
<div class="account-header">
    <div class="header-content">
        <h1>Financial Dashboard</h1>
        <div class="user-info">
            <span class="user-name">Welcome, User</span>
            <span class="last-updated">Last updated: <span id="lastUpdate">Just now</span></span>
        </div>
    </div>
</div>"#.to_string();

        let css = r#"
.account-header {
    background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
    padding: 2rem;
    color: white;
    border-radius: 8px;
    margin-bottom: 2rem;
    box-shadow: 0 4px 6px rgba(0,0,0,0.1);
}

.header-content h1 {
    margin: 0 0 0.5rem 0;
    font-size: 2rem;
}

.user-info {
    display: flex;
    justify-content: space-between;
    align-items: center;
    font-size: 0.9rem;
    opacity: 0.9;
}"#.to_string();

        let js = r#"
setInterval(() => {
    document.getElementById('lastUpdate').textContent = new Date().toLocaleTimeString();
}, 1000);"#.to_string();

        (html, css, js)
    }

    fn generate_balance_cards(&self) -> (String, String, String) {
        let html = r#"
<div class="balance-cards">
    <div class="balance-card checking">
        <div class="card-label">Checking Account</div>
        <div class="card-amount">$12,345.67</div>
        <div class="card-change positive">+2.5% this month</div>
    </div>
    <div class="balance-card savings">
        <div class="card-label">Savings Account</div>
        <div class="card-amount">$45,678.90</div>
        <div class="card-change positive">+5.2% this month</div>
    </div>
    <div class="balance-card investment">
        <div class="card-label">Investment Portfolio</div>
        <div class="card-amount">$123,456.78</div>
        <div class="card-change negative">-1.3% this month</div>
    </div>
</div>"#.to_string();

        let css = r#"
.balance-cards {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(250px, 1fr));
    gap: 1.5rem;
    margin-bottom: 2rem;
}

.balance-card {
    background: white;
    padding: 1.5rem;
    border-radius: 8px;
    box-shadow: 0 2px 4px rgba(0,0,0,0.1);
    transition: transform 0.2s;
}

.balance-card:hover {
    transform: translateY(-4px);
    box-shadow: 0 4px 8px rgba(0,0,0,0.15);
}

.card-label {
    font-size: 0.9rem;
    color: #666;
    margin-bottom: 0.5rem;
}

.card-amount {
    font-size: 2rem;
    font-weight: bold;
    color: #333;
    margin-bottom: 0.5rem;
}

.card-change {
    font-size: 0.85rem;
}

.card-change.positive {
    color: #10b981;
}

.card-change.negative {
    color: #ef4444;
}"#.to_string();

        (html, css, String::new())
    }

    fn generate_transaction_list(&self) -> (String, String, String) {
        let html = r#"
<div class="transactions-section">
    <h2>Recent Transactions</h2>
    <div class="transaction-list">
        <div class="transaction">
            <div class="transaction-icon">💳</div>
            <div class="transaction-details">
                <div class="transaction-name">Grocery Store</div>
                <div class="transaction-date">Today, 2:30 PM</div>
            </div>
            <div class="transaction-amount negative">-$87.45</div>
        </div>
        <div class="transaction">
            <div class="transaction-icon">💰</div>
            <div class="transaction-details">
                <div class="transaction-name">Salary Deposit</div>
                <div class="transaction-date">Yesterday</div>
            </div>
            <div class="transaction-amount positive">+$5,200.00</div>
        </div>
        <div class="transaction">
            <div class="transaction-icon">🏪</div>
            <div class="transaction-details">
                <div class="transaction-name">Online Shopping</div>
                <div class="transaction-date">2 days ago</div>
            </div>
            <div class="transaction-amount negative">-$234.56</div>
        </div>
    </div>
</div>"#.to_string();

        let css = r#"
.transactions-section {
    background: white;
    padding: 1.5rem;
    border-radius: 8px;
    box-shadow: 0 2px 4px rgba(0,0,0,0.1);
    margin-bottom: 2rem;
}

.transactions-section h2 {
    margin: 0 0 1rem 0;
    font-size: 1.5rem;
}

.transaction-list {
    display: flex;
    flex-direction: column;
    gap: 1rem;
}

.transaction {
    display: flex;
    align-items: center;
    padding: 1rem;
    border-radius: 4px;
    transition: background 0.2s;
}

.transaction:hover {
    background: #f9fafb;
}

.transaction-icon {
    font-size: 2rem;
    margin-right: 1rem;
}

.transaction-details {
    flex: 1;
}

.transaction-name {
    font-weight: 500;
    margin-bottom: 0.25rem;
}

.transaction-date {
    font-size: 0.85rem;
    color: #666;
}

.transaction-amount {
    font-weight: bold;
    font-size: 1.1rem;
}

.transaction-amount.positive {
    color: #10b981;
}

.transaction-amount.negative {
    color: #ef4444;
}"#.to_string();

        (html, css, String::new())
    }

    fn generate_portfolio_table(&self) -> (String, String, String) {
        let html = r#"
<div class="portfolio-section">
    <h2>Stock Portfolio</h2>
    <table class="portfolio-table">
        <thead>
            <tr>
                <th>Symbol</th>
                <th>Shares</th>
                <th>Price</th>
                <th>Value</th>
                <th>Change</th>
            </tr>
        </thead>
        <tbody>
            <tr>
                <td class="symbol">AAPL</td>
                <td>50</td>
                <td>$182.45</td>
                <td>$9,122.50</td>
                <td class="positive">+2.3%</td>
            </tr>
            <tr>
                <td class="symbol">GOOGL</td>
                <td>25</td>
                <td>$142.15</td>
                <td>$3,553.75</td>
                <td class="positive">+1.8%</td>
            </tr>
            <tr>
                <td class="symbol">TSLA</td>
                <td>15</td>
                <td>$245.30</td>
                <td>$3,679.50</td>
                <td class="negative">-3.2%</td>
            </tr>
        </tbody>
    </table>
</div>"#.to_string();

        let css = r#"
.portfolio-section {
    background: white;
    padding: 1.5rem;
    border-radius: 8px;
    box-shadow: 0 2px 4px rgba(0,0,0,0.1);
    margin-bottom: 2rem;
}

.portfolio-section h2 {
    margin: 0 0 1rem 0;
    font-size: 1.5rem;
}

.portfolio-table {
    width: 100%;
    border-collapse: collapse;
}

.portfolio-table th {
    text-align: left;
    padding: 0.75rem;
    background: #f9fafb;
    font-weight: 600;
    border-bottom: 2px solid #e5e7eb;
}

.portfolio-table td {
    padding: 0.75rem;
    border-bottom: 1px solid #e5e7eb;
}

.portfolio-table tr:hover {
    background: #f9fafb;
}

.symbol {
    font-weight: 600;
    color: #4f46e5;
}

.positive {
    color: #10b981;
    font-weight: 500;
}

.negative {
    color: #ef4444;
    font-weight: 500;
}"#.to_string();

        (html, css, String::new())
    }

    fn generate_chart(&self) -> (String, String, String) {
        let html = r#"
<div class="chart-section">
    <h2>Portfolio Performance</h2>
    <canvas id="performanceChart" width="800" height="400"></canvas>
</div>"#.to_string();

        let css = r#"
.chart-section {
    background: white;
    padding: 1.5rem;
    border-radius: 8px;
    box-shadow: 0 2px 4px rgba(0,0,0,0.1);
}

.chart-section h2 {
    margin: 0 0 1rem 0;
    font-size: 1.5rem;
}

#performanceChart {
    max-width: 100%;
    height: auto;
}"#.to_string();

        let js = r#"
const ctx = document.getElementById('performanceChart').getContext('2d');
const gradient = ctx.createLinearGradient(0, 0, 0, 400);
gradient.addColorStop(0, 'rgba(102, 126, 234, 0.5)');
gradient.addColorStop(1, 'rgba(118, 75, 162, 0.1)');

ctx.fillStyle = gradient;
ctx.fillRect(0, 0, 800, 400);

ctx.strokeStyle = '#667eea';
ctx.lineWidth = 3;
ctx.beginPath();
const points = [50, 120, 100, 180, 150, 200, 170, 250];
points.forEach((y, i) => {
    const x = (i / (points.length - 1)) * 800;
    if (i === 0) ctx.moveTo(x, 400 - y);
    else ctx.lineTo(x, 400 - y);
});
ctx.stroke();"#.to_string();

        (html, css, js)
    }

    fn generate_generic_header(&self) -> (String, String, String) {
        (
            "<div class=\"header\"><h1>Dashboard</h1></div>".to_string(),
            ".header { padding: 2rem; background: #667eea; color: white; }".to_string(),
            String::new(),
        )
    }

    fn generate_generic_content(&self) -> (String, String, String) {
        (
            "<div class=\"content\"><p>Main content area</p></div>".to_string(),
            ".content { padding: 2rem; }".to_string(),
            String::new(),
        )
    }

    fn generate_generic_footer(&self) -> (String, String, String) {
        (
            "<div class=\"footer\"><p>© 2025 Dashboard</p></div>".to_string(),
            ".footer { padding: 1rem; text-align: center; color: #666; }".to_string(),
            String::new(),
        )
    }

    pub async fn get_all_parts(&self) -> Vec<ComponentPart> {
        self.parts.read().await.clone()
    }

    pub async fn combine_parts(&self) -> (String, String, String) {
        let parts = self.get_all_parts().await;

        let html = parts.iter().map(|p| p.html.as_str()).collect::<Vec<_>>().join("\n");
        let css = parts.iter().map(|p| p.css.as_str()).collect::<Vec<_>>().join("\n");
        let js = parts
            .iter()
            .map(|p| p.javascript.as_str())
            .collect::<Vec<_>>()
            .join("\n");

        (html, css, js)
    }
}

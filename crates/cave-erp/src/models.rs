// SPDX-License-Identifier: AGPL-3.0-or-later
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ===== Core Shared Models =====

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Active,
    Inactive,
    Archived,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Company {
    pub id: Uuid,
    pub name: String,
    pub legal_name: Option<String>,
    pub vat_id: Option<String>,
    pub street: Option<String>,
    pub city: Option<String>,
    pub zip_code: Option<String>,
    pub country: Option<String>,
    pub phone: Option<String>,
    pub email: Option<String>,
    pub website: Option<String>,
    pub currency_id: String, // ISO 4217 code, e.g., "USD"
    pub status: Status,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Currency {
    pub code: String, // e.g., "USD", "EUR", "TRY"
    pub name: String,
    pub symbol: String,
    pub rate_to_base: f64, // Rate to EUR (base)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Address {
    pub street: String,
    pub city: String,
    pub state: Option<String>,
    pub zip_code: String,
    pub country: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Tag {
    Vip,
    LongTermPartner,
    AtRisk,
    NewCustomer,
    Custom(String),
}

// ===== HR Models =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Employee {
    pub id: Uuid,
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub department_id: Uuid,
    pub job_title: String,
    pub manager_id: Option<Uuid>,
    pub hire_date: DateTime<Utc>,
    pub termination_date: Option<DateTime<Utc>>,
    pub status: EmployeeStatus,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum EmployeeStatus {
    Active,
    OnLeave,
    Terminated,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Department {
    pub id: Uuid,
    pub name: String,
    pub manager_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Contract {
    pub id: Uuid,
    pub employee_id: Uuid,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub salary: f64,
    pub currency: String,
    pub contract_type: ContractType,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ContractType {
    Permanent,
    Fixed,
    Intern,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Leave {
    pub id: Uuid,
    pub employee_id: Uuid,
    pub leave_type: String,
    pub start: DateTime<Utc>,
    pub end: DateTime<Utc>,
    pub status: LeaveStatus,
    pub approver_id: Option<Uuid>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LeaveStatus {
    Pending,
    Approved,
    Rejected,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Timesheet {
    pub id: Uuid,
    pub employee_id: Uuid,
    pub project_id: Option<Uuid>,
    pub date: DateTime<Utc>,
    pub hours: f64,
    pub task_id: Option<Uuid>,
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
}

// ===== Recruitment Models =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: Uuid,
    pub title: String,
    pub department_id: Uuid,
    pub description: String,
    pub state: JobState,
    pub posted_at: DateTime<Utc>,
    pub headcount_target: u32,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobState {
    Draft,
    Open,
    Closed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Applicant {
    pub id: Uuid,
    pub job_id: Uuid,
    pub name: String,
    pub email: String,
    pub phone: Option<String>,
    pub resume_url: Option<String>,
    pub source: String,
    pub stage_id: Uuid,
    pub score: u8,
    pub rating: u8, // 1-5
    pub status: ApplicantStatus,
    pub applied_at: DateTime<Utc>,
    pub recruiter_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ApplicantStatus {
    Active,
    Hired,
    Rejected,
    Withdrawn,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecruitmentStage {
    pub id: Uuid,
    pub name: String,
    pub order: u32,
    pub is_won: bool,
    pub is_lost: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Interview {
    pub id: Uuid,
    pub applicant_id: Uuid,
    pub scheduled_at: DateTime<Utc>,
    pub interviewer_id: Uuid,
    pub mode: InterviewMode,
    pub feedback: Option<String>,
    pub outcome: InterviewOutcome,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InterviewMode {
    Onsite,
    Video,
    Phone,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InterviewOutcome {
    Pending,
    Pass,
    Fail,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Offer {
    pub id: Uuid,
    pub applicant_id: Uuid,
    pub salary: f64,
    pub currency: String,
    pub start_date: DateTime<Utc>,
    pub state: OfferState,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OfferState {
    Draft,
    Sent,
    Accepted,
    Rejected,
    Expired,
}

// ===== CRM Models =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lead {
    pub id: Uuid,
    pub name: String,
    pub contact_name: String,
    pub email: String,
    pub phone: Option<String>,
    pub company: String,
    pub source: String,
    pub status: LeadStatus,
    pub created_at: DateTime<Utc>,
    pub assigned_to: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum LeadStatus {
    New,
    Qualified,
    Unqualified,
    Converted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Opportunity {
    pub id: Uuid,
    pub name: String,
    pub stage_id: Uuid,
    pub amount: f64,
    pub currency: String,
    pub close_date: DateTime<Utc>,
    pub probability: u8,
    pub partner_id: Uuid,
    pub owner_id: Uuid,
    pub status: OpportunityStatus,
    pub state_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum OpportunityStatus {
    Open,
    Won,
    Lost,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CrmStage {
    pub id: Uuid,
    pub name: String,
    pub order: u32,
    pub probability: u8,
    pub is_won: bool,
    pub is_lost: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Activity {
    pub id: Uuid,
    pub entity_type: ActivityEntityType,
    pub entity_id: Uuid,
    pub activity_type: ActivityType,
    pub due_at: DateTime<Utc>,
    pub note: Option<String>,
    pub status: ActivityStatus,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityEntityType {
    Lead,
    Opportunity,
    Partner,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityType {
    Call,
    Email,
    Meeting,
    Task,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ActivityStatus {
    Planned,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Partner {
    pub id: Uuid,
    pub name: String,
    pub is_customer: bool,
    pub is_supplier: bool,
    pub email: String,
    pub phone: Option<String>,
    pub billing_address: Option<Address>,
    pub shipping_address: Option<Address>,
    pub tags: Vec<String>,
    pub created_at: DateTime<Utc>,
}

// ===== Sales Models =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaleOrder {
    pub id: Uuid,
    pub number: String,
    pub partner_id: Uuid,
    pub lines: Vec<SaleOrderLine>,
    pub state: SaleOrderState,
    pub created_at: DateTime<Utc>,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub delivery_date: Option<DateTime<Utc>>,
    pub amount_total: f64,
    pub currency: String,
    pub salesperson_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SaleOrderLine {
    pub id: Uuid,
    pub product_id: Uuid,
    pub name: String,
    pub quantity: f64,
    pub unit_price: f64,
    pub tax_ids: Vec<Uuid>,
    pub discount_pct: f64,
    pub subtotal: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SaleOrderState {
    Draft,
    Sent,
    Confirmed,
    Cancelled,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Delivery {
    pub id: Uuid,
    pub sale_order_id: Uuid,
    pub state: DeliveryState,
    pub scheduled: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub tracking_ref: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryState {
    Pending,
    InProgress,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Quotation {
    pub id: Uuid,
    pub number: String,
    pub partner_id: Uuid,
    pub lines: Vec<QuotationLine>,
    pub state: QuotationState,
    pub created_at: DateTime<Utc>,
    pub sent_at: Option<DateTime<Utc>>,
    pub amount_total: f64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotationLine {
    pub id: Uuid,
    pub product_id: Uuid,
    pub name: String,
    pub quantity: f64,
    pub unit_price: f64,
    pub tax_ids: Vec<Uuid>,
    pub discount_pct: f64,
    pub subtotal: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum QuotationState {
    Draft,
    Sent,
    Accepted,
    Rejected,
}

// ===== Purchase Models =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurchaseOrder {
    pub id: Uuid,
    pub number: String,
    pub supplier_id: Uuid,
    pub lines: Vec<PurchaseOrderLine>,
    pub state: PurchaseOrderState,
    pub created_at: DateTime<Utc>,
    pub received_at: Option<DateTime<Utc>>,
    pub amount_total: f64,
    pub currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PurchaseOrderLine {
    pub id: Uuid,
    pub product_id: Uuid,
    pub name: String,
    pub quantity: f64,
    pub unit_cost: f64,
    pub tax_ids: Vec<Uuid>,
    pub subtotal: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PurchaseOrderState {
    Draft,
    Sent,
    Confirmed,
    Received,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rfq {
    pub id: Uuid,
    pub supplier_id: Uuid,
    pub lines: Vec<RfqLine>,
    pub state: RfqState,
    pub requested_by: Uuid,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RfqLine {
    pub product_id: Uuid,
    pub name: String,
    pub quantity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum RfqState {
    Draft,
    Sent,
    Responded,
    Expired,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub id: Uuid,
    pub po_id: Uuid,
    pub lines: Vec<ReceiptLine>,
    pub received_at: DateTime<Utc>,
    pub receiver_id: Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReceiptLine {
    pub product_id: Uuid,
    pub quantity_received: f64,
}

// ===== Inventory Models =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Product {
    pub id: Uuid,
    pub sku: String,
    pub name: String,
    pub category_id: Uuid,
    pub unit_of_measure: UnitOfMeasure,
    pub price: f64,
    pub cost: f64,
    pub is_purchasable: bool,
    pub is_sellable: bool,
    pub tracked_by: TrackingType,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum UnitOfMeasure {
    Piece,
    Kg,
    Litre,
    Hour,
    Box,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TrackingType {
    None,
    Lot,
    Serial,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: Uuid,
    pub name: String,
    pub parent_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Warehouse {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    pub address: Option<Address>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockLocation {
    pub id: Uuid,
    pub warehouse_id: Uuid,
    pub name: String,
    pub is_internal: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StockMove {
    pub id: Uuid,
    pub product_id: Uuid,
    pub qty: f64,
    pub from_location_id: Uuid,
    pub to_location_id: Uuid,
    pub state: StockMoveState,
    pub lot_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub done_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum StockMoveState {
    Draft,
    Reserved,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lot {
    pub id: Uuid,
    pub product_id: Uuid,
    pub number: String,
    pub expiry_date: Option<DateTime<Utc>>,
    pub qty: f64,
    pub created_at: DateTime<Utc>,
}

// ===== Accounting Models =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Journal {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub kind: JournalKind,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JournalKind {
    Sales,
    Purchase,
    Bank,
    Cash,
    General,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    pub id: Uuid,
    pub code: String,
    pub name: String,
    pub account_type: AccountType,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AccountType {
    Asset,
    Liability,
    Equity,
    Income,
    Expense,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEntry {
    pub id: Uuid,
    pub journal_id: Uuid,
    pub date: DateTime<Utc>,
    pub reference: String,
    pub lines: Vec<JournalLine>,
    pub state: JournalEntryState,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalLine {
    pub account_id: Uuid,
    pub debit: f64,
    pub credit: f64,
    pub partner_id: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JournalEntryState {
    Draft,
    Posted,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tax {
    pub id: Uuid,
    pub name: String,
    pub pct: f64,
    pub kind: TaxKind,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaxKind {
    Sale,
    Purchase,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Invoice {
    pub id: Uuid,
    pub number: String,
    pub partner_id: Uuid,
    pub kind: InvoiceKind,
    pub journal_id: Uuid,
    pub lines: Vec<InvoiceLine>,
    pub amount_total: f64,
    pub state: InvoiceState,
    pub due_date: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceKind {
    Customer,
    Supplier,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum InvoiceState {
    Draft,
    Posted,
    Paid,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceLine {
    pub id: Uuid,
    pub product_id: Option<Uuid>,
    pub description: String,
    pub quantity: f64,
    pub unit_price: f64,
    pub tax_ids: Vec<Uuid>,
    pub subtotal: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Payment {
    pub id: Uuid,
    pub invoice_id: Uuid,
    pub amount: f64,
    pub date: DateTime<Utc>,
    pub method: PaymentMethod,
    pub state: PaymentState,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PaymentMethod {
    Cash,
    Bank,
    Card,
    Cheque,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PaymentState {
    Draft,
    Done,
    Cancelled,
}

// ===== Manufacturing Models =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Bom {
    pub id: Uuid,
    pub product_id: Uuid,
    pub components: Vec<BomComponent>,
    pub quantity: f64,
    pub routing_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BomComponent {
    pub product_id: Uuid,
    pub qty: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManufacturingOrder {
    pub id: Uuid,
    pub product_id: Uuid,
    pub qty: f64,
    pub bom_id: Uuid,
    pub state: ManufacturingOrderState,
    pub scheduled_start: DateTime<Utc>,
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ManufacturingOrderState {
    Draft,
    Confirmed,
    InProgress,
    Done,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkOrder {
    pub id: Uuid,
    pub mo_id: Uuid,
    pub workcenter_id: Uuid,
    pub duration_min: u32,
    pub state: WorkOrderState,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum WorkOrderState {
    Pending,
    InProgress,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workcenter {
    pub id: Uuid,
    pub name: String,
    pub capacity: f64,
    pub oee: f64,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Routing {
    pub id: Uuid,
    pub name: String,
    pub operations: Vec<Operation>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Operation {
    pub workcenter_id: Uuid,
    pub duration_min: u32,
    pub description: String,
}

// ===== Project Models =====

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Project {
    pub id: Uuid,
    pub name: String,
    pub code: String,
    pub customer_id: Option<Uuid>,
    pub manager_id: Uuid,
    pub state: ProjectState,
    pub start: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub budget: f64,
    pub currency: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ProjectState {
    Active,
    OnHold,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: Uuid,
    pub project_id: Uuid,
    pub title: String,
    pub description: Option<String>,
    pub assignee_id: Option<Uuid>,
    pub priority: TaskPriority,
    pub state: TaskState,
    pub estimated_hours: f64,
    pub spent_hours: f64,
    pub deadline: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Urgent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Todo,
    InProgress,
    Review,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Milestone {
    pub id: Uuid,
    pub project_id: Uuid,
    pub name: String,
    pub due_date: DateTime<Utc>,
    pub state: MilestoneState,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum MilestoneState {
    Upcoming,
    Achieved,
    Missed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeEntry {
    pub id: Uuid,
    pub project_id: Uuid,
    pub task_id: Option<Uuid>,
    pub user_id: Uuid,
    pub date: DateTime<Utc>,
    pub hours: f64,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
}

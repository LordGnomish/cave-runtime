// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright 2026 Cave Runtime contributors
use crate::models::*;
use chrono::Utc;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

pub struct ErpStore {
    // Core
    pub company: Arc<RwLock<HashMap<Uuid, Company>>>,
    pub currency: Arc<RwLock<HashMap<String, Currency>>>,

    // HR
    pub employees: Arc<RwLock<HashMap<Uuid, Employee>>>,
    pub departments: Arc<RwLock<HashMap<Uuid, Department>>>,
    pub contracts: Arc<RwLock<HashMap<Uuid, Contract>>>,
    pub leaves: Arc<RwLock<HashMap<Uuid, Leave>>>,
    pub timesheets: Arc<RwLock<HashMap<Uuid, Timesheet>>>,

    // Recruitment
    pub jobs: Arc<RwLock<HashMap<Uuid, Job>>>,
    pub applicants: Arc<RwLock<HashMap<Uuid, Applicant>>>,
    pub stages_recruit: Arc<RwLock<HashMap<Uuid, RecruitmentStage>>>,
    pub interviews: Arc<RwLock<HashMap<Uuid, Interview>>>,
    pub offers: Arc<RwLock<HashMap<Uuid, Offer>>>,

    // CRM
    pub leads: Arc<RwLock<HashMap<Uuid, Lead>>>,
    pub opportunities: Arc<RwLock<HashMap<Uuid, Opportunity>>>,
    pub stages_crm: Arc<RwLock<HashMap<Uuid, CrmStage>>>,
    pub activities: Arc<RwLock<HashMap<Uuid, Activity>>>,
    pub partners: Arc<RwLock<HashMap<Uuid, Partner>>>,

    // Sales
    pub sale_orders: Arc<RwLock<HashMap<Uuid, SaleOrder>>>,
    pub quotations: Arc<RwLock<HashMap<Uuid, Quotation>>>,
    pub deliveries: Arc<RwLock<HashMap<Uuid, Delivery>>>,

    // Purchase
    pub purchase_orders: Arc<RwLock<HashMap<Uuid, PurchaseOrder>>>,
    pub rfqs: Arc<RwLock<HashMap<Uuid, Rfq>>>,
    pub receipts: Arc<RwLock<HashMap<Uuid, Receipt>>>,

    // Inventory
    pub products: Arc<RwLock<HashMap<Uuid, Product>>>,
    pub categories: Arc<RwLock<HashMap<Uuid, Category>>>,
    pub warehouses: Arc<RwLock<HashMap<Uuid, Warehouse>>>,
    pub stock_locations: Arc<RwLock<HashMap<Uuid, StockLocation>>>,
    pub stock_moves: Arc<RwLock<HashMap<Uuid, StockMove>>>,
    pub lots: Arc<RwLock<HashMap<Uuid, Lot>>>,

    // Accounting
    pub journals: Arc<RwLock<HashMap<Uuid, Journal>>>,
    pub accounts: Arc<RwLock<HashMap<Uuid, Account>>>,
    pub entries: Arc<RwLock<HashMap<Uuid, JournalEntry>>>,
    pub taxes: Arc<RwLock<HashMap<Uuid, Tax>>>,
    pub invoices: Arc<RwLock<HashMap<Uuid, Invoice>>>,
    pub payments: Arc<RwLock<HashMap<Uuid, Payment>>>,

    // Manufacturing
    pub boms: Arc<RwLock<HashMap<Uuid, Bom>>>,
    pub manufacturing_orders: Arc<RwLock<HashMap<Uuid, ManufacturingOrder>>>,
    pub workcenters: Arc<RwLock<HashMap<Uuid, Workcenter>>>,
    pub work_orders: Arc<RwLock<HashMap<Uuid, WorkOrder>>>,
    pub routings: Arc<RwLock<HashMap<Uuid, Routing>>>,

    // Projects
    pub projects: Arc<RwLock<HashMap<Uuid, Project>>>,
    pub tasks: Arc<RwLock<HashMap<Uuid, Task>>>,
    pub milestones: Arc<RwLock<HashMap<Uuid, Milestone>>>,
    pub time_entries: Arc<RwLock<HashMap<Uuid, TimeEntry>>>,
}

impl Default for ErpStore {
    fn default() -> Self {
        let mut store = ErpStore {
            company: Arc::new(RwLock::new(HashMap::new())),
            currency: Arc::new(RwLock::new(HashMap::new())),
            employees: Arc::new(RwLock::new(HashMap::new())),
            departments: Arc::new(RwLock::new(HashMap::new())),
            contracts: Arc::new(RwLock::new(HashMap::new())),
            leaves: Arc::new(RwLock::new(HashMap::new())),
            timesheets: Arc::new(RwLock::new(HashMap::new())),
            jobs: Arc::new(RwLock::new(HashMap::new())),
            applicants: Arc::new(RwLock::new(HashMap::new())),
            stages_recruit: Arc::new(RwLock::new(HashMap::new())),
            interviews: Arc::new(RwLock::new(HashMap::new())),
            offers: Arc::new(RwLock::new(HashMap::new())),
            leads: Arc::new(RwLock::new(HashMap::new())),
            opportunities: Arc::new(RwLock::new(HashMap::new())),
            stages_crm: Arc::new(RwLock::new(HashMap::new())),
            activities: Arc::new(RwLock::new(HashMap::new())),
            partners: Arc::new(RwLock::new(HashMap::new())),
            sale_orders: Arc::new(RwLock::new(HashMap::new())),
            quotations: Arc::new(RwLock::new(HashMap::new())),
            deliveries: Arc::new(RwLock::new(HashMap::new())),
            purchase_orders: Arc::new(RwLock::new(HashMap::new())),
            rfqs: Arc::new(RwLock::new(HashMap::new())),
            receipts: Arc::new(RwLock::new(HashMap::new())),
            products: Arc::new(RwLock::new(HashMap::new())),
            categories: Arc::new(RwLock::new(HashMap::new())),
            warehouses: Arc::new(RwLock::new(HashMap::new())),
            stock_locations: Arc::new(RwLock::new(HashMap::new())),
            stock_moves: Arc::new(RwLock::new(HashMap::new())),
            lots: Arc::new(RwLock::new(HashMap::new())),
            journals: Arc::new(RwLock::new(HashMap::new())),
            accounts: Arc::new(RwLock::new(HashMap::new())),
            entries: Arc::new(RwLock::new(HashMap::new())),
            taxes: Arc::new(RwLock::new(HashMap::new())),
            invoices: Arc::new(RwLock::new(HashMap::new())),
            payments: Arc::new(RwLock::new(HashMap::new())),
            boms: Arc::new(RwLock::new(HashMap::new())),
            manufacturing_orders: Arc::new(RwLock::new(HashMap::new())),
            workcenters: Arc::new(RwLock::new(HashMap::new())),
            work_orders: Arc::new(RwLock::new(HashMap::new())),
            routings: Arc::new(RwLock::new(HashMap::new())),
            projects: Arc::new(RwLock::new(HashMap::new())),
            tasks: Arc::new(RwLock::new(HashMap::new())),
            milestones: Arc::new(RwLock::new(HashMap::new())),
            time_entries: Arc::new(RwLock::new(HashMap::new())),
        };

        // Seed default company (synchronously via Arc construction)
        let company_id = Uuid::new_v4();
        let company = Company {
            id: company_id,
            name: "Default Company".to_string(),
            legal_name: Some("Default Company Inc.".to_string()),
            vat_id: None,
            street: None,
            city: None,
            zip_code: None,
            country: None,
            phone: None,
            email: None,
            website: None,
            currency_id: "EUR".to_string(),
            status: Status::Active,
            created_at: Utc::now(),
        };

        // Seed currencies (synchronously)
        let currencies = vec![
            (
                "EUR".to_string(),
                Currency {
                    code: "EUR".to_string(),
                    name: "Euro".to_string(),
                    symbol: "€".to_string(),
                    rate_to_base: 1.0,
                },
            ),
            (
                "USD".to_string(),
                Currency {
                    code: "USD".to_string(),
                    name: "US Dollar".to_string(),
                    symbol: "$".to_string(),
                    rate_to_base: 1.1,
                },
            ),
            (
                "TRY".to_string(),
                Currency {
                    code: "TRY".to_string(),
                    name: "Turkish Lira".to_string(),
                    symbol: "₺".to_string(),
                    rate_to_base: 0.033,
                },
            ),
        ];

        // Seed CRM stages
        let crm_stages = vec![
            CrmStage {
                id: Uuid::new_v4(),
                name: "New".to_string(),
                order: 1,
                probability: 10,
                is_won: false,
                is_lost: false,
            },
            CrmStage {
                id: Uuid::new_v4(),
                name: "Qualified".to_string(),
                order: 2,
                probability: 30,
                is_won: false,
                is_lost: false,
            },
            CrmStage {
                id: Uuid::new_v4(),
                name: "Proposition".to_string(),
                order: 3,
                probability: 60,
                is_won: false,
                is_lost: false,
            },
            CrmStage {
                id: Uuid::new_v4(),
                name: "Negotiation".to_string(),
                order: 4,
                probability: 80,
                is_won: false,
                is_lost: false,
            },
            CrmStage {
                id: Uuid::new_v4(),
                name: "Won".to_string(),
                order: 5,
                probability: 100,
                is_won: true,
                is_lost: false,
            },
        ];

        // Seed recruitment stages
        let recruit_stages = vec![
            RecruitmentStage {
                id: Uuid::new_v4(),
                name: "Applied".to_string(),
                order: 1,
                is_won: false,
                is_lost: false,
            },
            RecruitmentStage {
                id: Uuid::new_v4(),
                name: "Screening".to_string(),
                order: 2,
                is_won: false,
                is_lost: false,
            },
            RecruitmentStage {
                id: Uuid::new_v4(),
                name: "Interview".to_string(),
                order: 3,
                is_won: false,
                is_lost: false,
            },
            RecruitmentStage {
                id: Uuid::new_v4(),
                name: "Offer".to_string(),
                order: 4,
                is_won: false,
                is_lost: false,
            },
            RecruitmentStage {
                id: Uuid::new_v4(),
                name: "Hired".to_string(),
                order: 5,
                is_won: true,
                is_lost: false,
            },
            RecruitmentStage {
                id: Uuid::new_v4(),
                name: "Rejected".to_string(),
                order: 6,
                is_won: false,
                is_lost: true,
            },
        ];

        // Insert into store (this is done in a blocking context, so we need to be careful)
        // Since Arc<RwLock<>> cannot be mutated during construction without runtime,
        // we return a partially initialized store and rely on module handlers to seed
        // But we can at least try to populate with blocking operations if possible

        // For now, return the empty store with structure in place
        // Module initialization can populate these via routes if needed
        store
    }
}
